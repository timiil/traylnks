//! UI Automation stays on one COM MTA thread. SelectionItem selects a tab
//! directly, without keyboard shortcuts or any input to a terminal session.
use super::graphics::CaptureDevice;
use super::model::{Bounds, PaneSnap, TabSnap, WindowSnap};
use chrono::Local;
use std::path::Path;
use std::time::{Duration, Instant};
use windows::core::{Interface, BOOL};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::*;

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}

/// A borrowed UIA element holds the tab's runtime identity, even when titles
/// change or two tabs have the same title. Never re-find tabs by their name.
struct Tab {
    element: IUIAutomationElement,
    selected: bool,
    title: String,
}

struct CaptureContext<'a> {
    automation: &'a IUIAutomation,
    device: &'a Result<CaptureDevice, String>,
    directory: &'a Path,
}

struct RestoreWindow {
    hwnd: HWND,
    placement: WINDOWPLACEMENT,
    selected: Option<IUIAutomationElement>,
    focus: Option<IUIAutomationElement>,
    restored: bool,
}

impl RestoreWindow {
    unsafe fn restore(&mut self) -> Vec<String> {
        if self.restored {
            return Vec::new();
        }
        self.restored = true;
        let mut warnings = Vec::new();
        if !IsWindow(Some(self.hwnd)).as_bool() {
            return warnings;
        }
        if let Some(selected) = &self.selected {
            let result = selected
                .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                    UIA_SelectionItemPatternId,
                )
                .and_then(|p| p.Select());
            if let Err(error) = result {
                warnings.push(format!("Restore selected tab: {error}"));
            }
        }
        if let Some(focus) = &self.focus {
            if let Err(error) = focus.SetFocus() {
                warnings.push(format!("Restore pane focus: {error}"));
            }
        }
        if let Err(error) = SetWindowPlacement(self.hwnd, &self.placement) {
            warnings.push(format!("Restore window state: {error}"));
        }
        warnings
    }
}

impl Drop for RestoreWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = self.restore();
        }
    }
}

struct RestoreForeground {
    hwnd: HWND,
    focus: Option<IUIAutomationElement>,
    restored: bool,
}
impl RestoreForeground {
    unsafe fn restore(&mut self) -> Option<String> {
        if self.restored {
            return None;
        }
        self.restored = true;
        if !IsWindow(Some(self.hwnd)).as_bool() {
            return None;
        }
        let mut error = None;
        if !SetForegroundWindow(self.hwnd).as_bool() {
            error = Some("Could not restore original foreground window".into());
        }
        if let Some(focus) = &self.focus {
            if let Err(e) = focus.SetFocus() {
                error = Some(format!("Restore original focus: {e}"));
            }
        }
        error
    }
}
impl Drop for RestoreForeground {
    fn drop(&mut self) {
        unsafe {
            let _ = self.restore();
        }
    }
}

pub fn allow_foreground() {
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }
}

pub fn visible_foreground() -> Option<isize> {
    unsafe {
        let hwnd = GetForegroundWindow();
        IsWindowVisible(hwnd).as_bool().then_some(hwnd.0 as isize)
    }
}

pub fn collect(
    directory: &Path,
    foreground_before_menu: isize,
    mut progress: impl FnMut(String),
) -> Result<(Vec<WindowSnap>, Vec<String>), String> {
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)
            .map_err(|e| format!("Initialize capture thread: {e}"))?;
        let _apartment = Apartment;
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("Initialize UI Automation: {e}"))?;
        let automation2: IUIAutomation2 = automation.cast().map_err(|e| e.to_string())?;
        automation2
            .SetConnectionTimeout(1500)
            .map_err(|e| e.to_string())?;
        automation2
            .SetTransactionTimeout(2000)
            .map_err(|e| e.to_string())?;
        let active = GetForegroundWindow();
        let previous = HWND(foreground_before_menu as *mut _);
        let hwnd = if !IsWindowVisible(active).as_bool() && IsWindow(Some(previous)).as_bool() {
            previous
        } else {
            active
        };
        let mut original_pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut original_pid));
        let mut original = RestoreForeground {
            hwnd,
            focus: automation.GetFocusedElement().ok().filter(|element| {
                element
                    .CurrentProcessId()
                    .map(|pid| pid as u32 == original_pid)
                    .unwrap_or(false)
            }),
            restored: false,
        };
        let mut handles: Vec<HWND> = Vec::new();
        EnumWindows(Some(enum_window), LPARAM(&mut handles as *mut _ as isize))
            .map_err(|e| format!("Find terminal windows: {e}"))?;
        if handles.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let device = CaptureDevice::new();
        let context = CaptureContext {
            automation: &automation,
            device: &device,
            directory,
        };
        let mut windows = Vec::new();
        let mut warnings = Vec::new();
        // Give the tray menu time to dismiss before activating another window.
        std::thread::sleep(Duration::from_millis(150));
        for (index, hwnd) in handles.into_iter().enumerate() {
            progress(format!("Capturing window {}…", index + 1));
            let mut title = [0u16; 4096];
            let length = GetWindowTextW(hwnd, &mut title).max(0) as usize;
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let mut record = WindowSnap {
                title: String::from_utf16_lossy(&title[..length]),
                process_id: pid,
                originally_minimized: IsIconic(hwnd).as_bool(),
                tabs: Vec::new(),
                error: None,
            };
            if let Err(error) = collect_window(
                &context,
                hwnd,
                index,
                &mut record,
                &mut warnings,
                &mut progress,
            ) {
                record.error = Some(error);
            }
            windows.push(record);
        }
        if let Some(error) = original.restore() {
            warnings.push(error);
        }
        Ok((windows, warnings))
    }
}

unsafe extern "system" fn enum_window(hwnd: HWND, param: LPARAM) -> BOOL {
    let mut class = [0u16; 128];
    let length = GetClassNameW(hwnd, &mut class).max(0) as usize;
    if String::from_utf16_lossy(&class[..length]) == "CASCADIA_HOSTING_WINDOW_CLASS"
        && IsWindowVisible(hwnd).as_bool()
    {
        (*(param.0 as *mut Vec<HWND>)).push(hwnd);
    }
    BOOL(1)
}

unsafe fn find_elements(
    automation: &IUIAutomation,
    root: &IUIAutomationElement,
    property: UIA_PROPERTY_ID,
    value: VARIANT,
) -> Result<Vec<IUIAutomationElement>, String> {
    let condition = automation
        .CreatePropertyCondition(property, &value)
        .map_err(|e| e.to_string())?;
    let array = root
        .FindAll(TreeScope_Descendants, &condition)
        .map_err(|e| e.to_string())?;
    let length = array.Length().map_err(|e| e.to_string())?;
    (0..length)
        .map(|i| array.GetElement(i).map_err(|e| e.to_string()))
        .collect()
}

unsafe fn collect_window(
    context: &CaptureContext<'_>,
    hwnd: HWND,
    window_index: usize,
    record: &mut WindowSnap,
    warnings: &mut Vec<String>,
    progress: &mut impl FnMut(String),
) -> Result<(), String> {
    let automation = context.automation;
    let device = context.device;
    let directory = context.directory;
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    GetWindowPlacement(hwnd, &mut placement).map_err(|e| format!("Read window state: {e}"))?;
    let mut restore = RestoreWindow {
        hwnd,
        placement,
        selected: None,
        focus: None,
        restored: false,
    };
    let result = (|| {
        if record.originally_minimized {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if !SetForegroundWindow(hwnd).as_bool() {
            return Err(
                "Cannot activate terminal window (it may require administrator access)".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(120));
        let root = automation
            .ElementFromHandle(hwnd)
            .map_err(|e| format!("Read terminal window: {e}"))?;
        let elements = find_elements(
            automation,
            &root,
            UIA_ControlTypePropertyId,
            VARIANT::from(UIA_TabItemControlTypeId.0),
        )?;
        let mut tabs = Vec::new();
        for element in elements {
            let pattern: IUIAutomationSelectionItemPattern = element
                .GetCurrentPatternAs(UIA_SelectionItemPatternId)
                .map_err(|e| format!("Read terminal tab: {e}"))?;
            let selected = pattern
                .CurrentIsSelected()
                .map_err(|e| e.to_string())?
                .as_bool();
            if selected {
                restore.selected = Some(element.clone());
            }
            tabs.push(Tab {
                title: element
                    .CurrentName()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|_| "(unavailable title)".into()),
                element,
                selected,
            });
        }
        if tabs.is_empty() {
            return Err("No terminal tabs exposed by UI Automation".into());
        }
        restore.focus = automation.GetFocusedElement().ok().filter(|e| {
            e.CurrentClassName()
                .map(|v| v == "TermControl")
                .unwrap_or(false)
        });
        let tab_count = tabs.len();
        for (tab_index, tab) in tabs.into_iter().enumerate() {
            progress(format!(
                "Window {} · tab {}/{}",
                window_index + 1,
                tab_index + 1,
                tab_count
            ));
            let mut captured = TabSnap {
                id: format!("w{:03}-t{:03}", window_index + 1, tab_index + 1),
                title: tab.title,
                index: tab_index,
                was_selected: tab.selected,
                captured_at: Local::now().to_rfc3339(),
                panes: Vec::new(),
                screenshot: false,
                screenshot_error: None,
                text_error: None,
            };
            match select_tab(&tab.element) {
                Err(error) => {
                    captured.screenshot_error = Some(error.clone());
                    captured.text_error = Some(error);
                }
                Ok(()) => {
                    // Start a new capture session only after selection and layout settle;
                    // no frame from the previous tab is kept in a shared frame pool.
                    std::thread::sleep(Duration::from_millis(180));
                    captured.captured_at = Local::now().to_rfc3339();
                    if let Ok(title) = tab.element.CurrentName() {
                        captured.title = title.to_string();
                    }
                    match find_elements(
                        automation,
                        &root,
                        UIA_ClassNamePropertyId,
                        VARIANT::from("TermControl"),
                    ) {
                        Ok(panes) => {
                            for pane in panes {
                                if pane
                                    .CurrentIsOffscreen()
                                    .map(|v| v.as_bool())
                                    .unwrap_or(false)
                                {
                                    continue;
                                }
                                captured.panes.push(read_pane(&pane));
                            }
                            if captured.panes.is_empty() {
                                captured.text_error = Some(
                                    "No visible terminal panes exposed by UI Automation".into(),
                                );
                            }
                        }
                        Err(error) => captured.text_error = Some(error),
                    }
                    let bounds = viewport_bounds(&captured.panes);
                    let image = device.as_ref().map_err(Clone::clone).and_then(|device| {
                        device.save(
                            hwnd,
                            bounds,
                            &directory.join(format!("{}.png", captured.id)),
                        )
                    });
                    match image {
                        Ok(()) => captured.screenshot = true,
                        Err(error) => captured.screenshot_error = Some(error),
                    }
                    if !tab
                        .element
                        .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                            UIA_SelectionItemPatternId,
                        )
                        .and_then(|p| p.CurrentIsSelected())
                        .map(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        // A user action or tab closure raced capture; never associate
                        // another tab's surface/text with the requested header.
                        if captured.screenshot {
                            let _ = std::fs::remove_file(
                                directory.join(format!("{}.png", captured.id)),
                            );
                        }
                        captured.screenshot = false;
                        captured.screenshot_error =
                            Some("Tab selection changed during capture; retry TakeSnap".into());
                        captured.text_error = captured.screenshot_error.clone();
                        captured.panes.clear();
                    }
                }
            }
            record.tabs.push(captured);
        }
        Ok(())
    })();
    warnings.extend(restore.restore());
    result
}

unsafe fn select_tab(element: &IUIAutomationElement) -> Result<(), String> {
    let pattern: IUIAutomationSelectionItemPattern = element
        .GetCurrentPatternAs(UIA_SelectionItemPatternId)
        .map_err(|e| e.to_string())?;
    pattern.Select().map_err(|e| format!("Select tab: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if pattern
            .CurrentIsSelected()
            .map_err(|e| e.to_string())?
            .as_bool()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("Timed out selecting tab".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

unsafe fn read_pane(element: &IUIAutomationElement) -> PaneSnap {
    let mut pane = PaneSnap {
        title: element
            .CurrentName()
            .map(|v| v.to_string())
            .unwrap_or_default(),
        bounds: Bounds::default(),
        text: None,
        error: None,
    };
    if let Ok(rect) = element.CurrentBoundingRectangle() {
        pane.bounds = Bounds {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        };
    }
    let result = (|| -> windows::core::Result<String> {
        let pattern: IUIAutomationTextPattern = element.GetCurrentPatternAs(UIA_TextPatternId)?;
        let ranges = pattern.GetVisibleRanges()?;
        let mut text = String::new();
        for index in 0..ranges.Length()? {
            text.push_str(&ranges.GetElement(index)?.GetText(-1)?.to_string());
        }
        Ok(text)
    })();
    match result {
        Ok(text) => pane.text = Some(text),
        Err(error) => pane.error = Some(format!("Read visible text: {error}")),
    }
    pane
}

fn viewport_bounds(panes: &[PaneSnap]) -> Option<Bounds> {
    panes
        .iter()
        .map(|p| p.bounds)
        .filter(|b| b.right > b.left && b.bottom > b.top)
        .reduce(|a, b| Bounds {
            left: a.left.min(b.left),
            top: a.top.min(b.top),
            right: a.right.max(b.right),
            bottom: a.bottom.max(b.bottom),
        })
}
