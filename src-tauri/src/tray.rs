//! Tray menu construction, rebuild, and click routing.
//!
//! `build_menu` converts a `MenuNode` tree into a Tauri `Menu` (folders →
//! `Submenu`, items → `IconMenuItem`, plus bottom utility items). `rebuild_now`
//! does the filesystem scan off the main thread, then hops to the main thread
//! to rebuild the menu (`set_menu`) — menu APIs have thread affinity on Windows.

use crate::menu_tree::{Kind, MenuNode};
use std::path::PathBuf;
use tauri::menu::{
    IconMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu,
};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Wry};

pub const TRAY_ID: &str = "main";
const LNK_PREFIX: &str = "lnk:";

/// Build the tray icon itself (called once at startup).
pub fn build_tray(app: &AppHandle<Wry>) -> tauri::Result<()> {
    let menu = build_menu(app, &current_tree(app))?;
    let icon = app
        .default_window_icon()
        .cloned()
        .unwrap_or_else(|| tauri::image::Image::new_owned(vec![0u8; 4], 1, 1));
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("TrayLnks")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .build(app)?;
    Ok(())
}

/// Scan the current watch path (off-main) and swap in a fresh menu (on-main).
/// No-op-safe: if no watch path, shows only the utility items.
pub fn rebuild_now(app: &AppHandle<Wry>) {
    let watch = crate::state::watch_path(app);
    let app2 = app.clone();
    std::thread::spawn(move || {
        let tree = match &watch {
            Some(wp) => {
                let hostname = crate::state::hostname(&app2);
                crate::menu_tree::build_tree(wp, &hostname)
            }
            None => empty_tree(),
        };
        let app3 = app2.clone();
        let _ = app2.run_on_main_thread(move || {
            match build_menu(&app3, &tree) {
                Ok(menu) => {
                    if let Some(tray) = app3.tray_by_id(TRAY_ID) {
                        if let Err(e) = tray.set_menu(Some(menu)) {
                            log::error!("set_menu failed: {e}");
                        }
                    }
                    crate::state::set_last_scan(&app3, std::time::SystemTime::now());
                }
                Err(e) => log::error!("build_menu failed: {e}"),
            }
        });
    });
}

/// Route a menu click. `.lnk` items carry their path in the id; the rest are
/// the fixed utility items.
pub fn handle_menu_event(app: &AppHandle<Wry>, event: tauri::menu::MenuEvent) {
    let id = event.id().as_ref();
    if let Some(path_str) = id.strip_prefix(LNK_PREFIX) {
        let path = PathBuf::from(path_str);
        if let Err(e) = crate::launch::open(&path) {
            log::error!("launch {} failed: {}", path.display(), e);
        }
        return;
    }
    match id {
        "refresh" => rebuild_now(app),
        "open_folder" => {
            if let Some(wp) = crate::state::watch_path(app) {
                if let Err(e) = crate::launch::open(&wp) {
                    log::error!("open launcher folder failed: {e}");
                }
            }
        }
        "settings" => crate::commands::show_settings(app),
        "exit" => app.exit(0),
        _ => {}
    }
}

fn current_tree(app: &AppHandle<Wry>) -> MenuNode {
    match crate::state::watch_path(app) {
        Some(wp) => {
            let hostname = crate::state::hostname(app);
            crate::menu_tree::build_tree(&wp, &hostname)
        }
        None => empty_tree(),
    }
}

fn empty_tree() -> MenuNode {
    MenuNode {
        kind: Kind::Folder,
        label: String::new(),
        lnk_path: None,
        icon_base: None,
        children: Vec::new(),
    }
}

/// Build the full menu: folder/item children, then a separator and the
/// fixed utility items (Refresh / Open Launcher Folder / Settings / Exit).
pub fn build_menu(app: &AppHandle<Wry>, tree: &MenuNode) -> tauri::Result<Menu<Wry>> {
    let mut owned: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();
    for child in &tree.children {
        owned.push(build_item(app, child)?);
    }
    owned.push(Box::new(PredefinedMenuItem::separator(app)?));
    owned.push(Box::new(MenuItem::with_id(
        app,
        "refresh",
        "Refresh",
        true,
        None::<&str>,
    )?));
    owned.push(Box::new(MenuItem::with_id(
        app,
        "open_folder",
        "Open Launcher Folder",
        true,
        None::<&str>,
    )?));
    owned.push(Box::new(MenuItem::with_id(
        app,
        "settings",
        "Settings",
        true,
        None::<&str>,
    )?));
    owned.push(Box::new(MenuItem::with_id(
        app,
        "exit",
        "Exit",
        true,
        None::<&str>,
    )?));

    let refs: Vec<&dyn IsMenuItem<Wry>> = owned.iter().map(|b| b.as_ref()).collect();
    Menu::with_items(app, &refs)
}

fn build_item(app: &AppHandle<Wry>, node: &MenuNode) -> tauri::Result<Box<dyn IsMenuItem<Wry>>> {
    match node.kind {
        Kind::Folder => {
            let mut children: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();
            for child in &node.children {
                children.push(build_item(app, child)?);
            }
            let child_refs: Vec<&dyn IsMenuItem<Wry>> =
                children.iter().map(|b| b.as_ref()).collect();
            let submenu = Submenu::with_items(app, node.label.as_str(), true, &child_refs)?;
            // optional folder icon (best-effort; ignored if it fails)
            if let Some(base) = &node.icon_base {
                if let Some(icon) = crate::icon::load_icon(base) {
                    let _ = submenu.set_icon(Some(icon));
                }
            }
            Ok(Box::new(submenu))
        }
        Kind::Item => {
            let icon = node.icon_base.as_ref().and_then(|p| crate::icon::load_icon(p));
            let id = lnk_id(node.lnk_path.as_deref());
            Ok(Box::new(IconMenuItem::with_id(
                app,
                id,
                node.label.as_str(),
                true,
                icon,
                None::<&str>,
            )?))
        }
    }
}

/// Encode the `.lnk` path into the menu-item id so click routing can recover it.
fn lnk_id(path: Option<&std::path::Path>) -> String {
    match path {
        Some(p) => format!("{LNK_PREFIX}{}", p.to_string_lossy()),
        None => format!("{LNK_PREFIX}<none>"),
    }
}
