//! Pure menu-tree model + recursive scan (PRD §4, §6, §7).
//!
//! This module has NO Tauri types — only filesystem logic — so it is unit-testable
//! and can run off the main thread. It applies:
//!   - whitelist rendering (folders + launchables `.lnk`/`.cmd`/`.ps1`; rest ignored)
//!   - `.station` short-circuit (a non-matching parent hides the whole subtree)
//!   - icon-base resolution (path without extension, for later icon lookup)
//!   - fault tolerance (unreadable dirs, bad file types → skip + log, never panic)

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Item,
}

#[derive(Debug, Clone)]
pub struct MenuNode {
    pub kind: Kind,
    /// Display label: folder name, or the launchable file's stem (extension stripped).
    pub label: String,
    /// `Some` for items — the absolute path to launch (`.lnk`/`.cmd`/`.ps1`).
    pub target_path: Option<PathBuf>,
    /// Path without extension, used to look up same-name icon files.
    pub icon_base: Option<PathBuf>,
    pub children: Vec<MenuNode>,
}

/// Hard cap to guard against pathological deep nesting.
const MAX_DEPTH: usize = 32;

/// File extensions rendered as launchable menu items (case-insensitive).
const LAUNCH_EXTS: &[&str] = &["lnk", "cmd", "ps1"];

/// Build the menu tree from `root`. The root node is always a Folder container;
/// its children are the filtered scan results.
pub fn build_tree(root: &Path, hostname_lower: &str) -> MenuNode {
    let label = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().to_string());
    let mut node = MenuNode {
        kind: Kind::Folder,
        label,
        target_path: None,
        icon_base: Some(root.to_path_buf()),
        children: Vec::new(),
    };
    // The root's own `.station` gates the whole menu (PRD §6).
    if !matches!(
        crate::station::verdict_for_dir(root, hostname_lower),
        crate::station::StationVerdict::Hide
    ) {
        fill_children(&mut node, root, hostname_lower, 0);
    }
    node
}

fn fill_children(parent: &mut MenuNode, dir: &Path, hostname_lower: &str, depth: usize) {
    if depth >= MAX_DEPTH {
        log::warn!(
            "max nesting depth {} reached at {}",
            MAX_DEPTH,
            dir.display()
        );
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            log::warn!("skip unreadable dir {}: {}", dir.display(), err);
            return;
        }
    };

    let mut items: Vec<MenuNode> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                log::warn!("file_type error at {}: {}", path.display(), e);
                continue;
            }
        };

        if ft.is_dir() {
            // A subdirectory's own `.station` decides whether IT is shown
            // (and its whole subtree). Non-matching → skip entirely.
            if matches!(
                crate::station::verdict_for_dir(&path, hostname_lower),
                crate::station::StationVerdict::Hide
            ) {
                continue;
            }
            let label = entry.file_name().to_string_lossy().to_string();
            let mut child = MenuNode {
                kind: Kind::Folder,
                label,
                target_path: None,
                icon_base: Some(path.clone()),
                children: Vec::new(),
            };
            fill_children(&mut child, &path, hostname_lower, depth + 1);
            items.push(child);
        } else if ft.is_file() && is_launchable(&path) {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            items.push(MenuNode {
                kind: Kind::Item,
                label: stem,
                target_path: Some(path.clone()),
                icon_base: Some(path.with_extension("")),
                children: Vec::new(),
            });
        }
        // everything else (.md, .txt, .bat, icon files, .station) is ignored silently
    }

    // Stable order: folders first, then items, each case-insensitive alphabetical.
    items.sort_by(|a, b| match (a.kind, b.kind) {
        (Kind::Folder, Kind::Item) => std::cmp::Ordering::Less,
        (Kind::Item, Kind::Folder) => std::cmp::Ordering::Greater,
        _ => a.label.to_lowercase().cmp(&b.label.to_lowercase()),
    });

    parent.children = items;
}

/// True if `path`'s extension is a launchable type (`.lnk`/`.cmd`/`.ps1`).
fn is_launchable(path: &Path) -> bool {
    path.extension()
        .map(|e| LAUNCH_EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "traylnks-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn whitelist_renders_launchables_and_skips_others() {
        let root = tmp();
        // launchables (render)
        fs::write(root.join("Terminal.lnk"), b"").unwrap();
        fs::write(root.join("build.cmd"), b"").unwrap();
        fs::write(root.join("run.ps1"), b"").unwrap();
        // non-launchables (do NOT render)
        fs::write(root.join("notes.txt"), b"").unwrap();
        fs::write(root.join("readme.md"), b"").unwrap();
        fs::write(root.join("old.bat"), b"").unwrap(); // .bat excluded by choice
        fs::write(root.join("Terminal.png"), b"").unwrap(); // icon, not an item
        fs::create_dir(root.join("Dev")).unwrap();

        let tree = build_tree(&root, "anyhost");
        let labels: Vec<&str> = tree.children.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"Dev"));
        assert!(labels.contains(&"Terminal"));
        assert!(labels.contains(&"build"));
        assert!(labels.contains(&"run"));
        assert!(!labels.contains(&"notes"));
        assert!(!labels.contains(&"readme"));
        assert!(!labels.contains(&"old"));
        assert!(!labels.contains(&"Terminal.png"));
    }

    #[test]
    fn station_hide_stops_subtree() {
        let root = tmp();
        fs::create_dir(root.join("Hidden")).unwrap();
        fs::write(root.join("Hidden").join(".station"), b"OTHER-HOST\n").unwrap();
        fs::write(root.join("Hidden").join("x.lnk"), b"").unwrap();

        let tree = build_tree(&root, "thishost");
        assert!(tree.children.iter().all(|c| c.label != "Hidden"));
    }

    #[test]
    fn empty_station_hides_dir() {
        let root = tmp();
        fs::create_dir(root.join("Ghost")).unwrap();
        fs::write(root.join("Ghost").join(".station"), b"").unwrap();
        fs::write(root.join("Ghost").join("y.lnk"), b"").unwrap();

        let tree = build_tree(&root, "thishost");
        assert!(tree.children.iter().all(|c| c.label != "Ghost"));
    }

    #[test]
    fn matching_station_shows_dir() {
        let root = tmp();
        fs::create_dir(root.join("Mine")).unwrap();
        fs::write(root.join("Mine").join(".station"), b"THISHOST\n# c\n").unwrap();
        fs::write(root.join("Mine").join("z.lnk"), b"").unwrap();

        let tree = build_tree(&root, "thishost");
        assert!(tree.children.iter().any(|c| c.label == "Mine"));
    }

    #[test]
    fn lnk_label_strips_extension_and_cjk_safe() {
        let root = tmp();
        fs::write(root.join("simulate2 工作区.lnk"), b"").unwrap();
        let tree = build_tree(&root, "h");
        assert!(tree.children.iter().any(|c| c.label == "simulate2 工作区"));
    }
}
