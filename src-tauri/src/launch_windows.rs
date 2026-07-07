//! Windows-native launch + foreground focus.
//!
//! Clicking a tray item launches the target and brings its window to the
//! foreground. Because the tray app is a non-foreground background process,
//! `SetForegroundWindow` alone often fails (window just flashes). The recipe:
//!   1. `AllowSetForegroundWindow(ASFW_ANY)` on the click thread (Tauri
//!      dispatches menu events on the main thread, which carries the
//!      "last input event" allowance that makes ASFW succeed).
//!   2. `ShellExecuteExW(SEE_MASK_NOCLOSEPROCESS)` → process handle → PID.
//!   3. Worker thread: poll `EnumWindows` for a visible window of that PID, then
//!      spoof an ALT keystroke + `SetForegroundWindow`/`ShowWindow`/`BringWindowToTop`.
//!
//! `.ps1` needs explicit invocation (`powershell.exe -File`) because its default
//! shell association opens an editor instead of running it. Console windows
//! (`.cmd`/`.ps1`) are owned by conhost, not the launched PID, so the window
//! poll finds nothing — but ASFW already lets the console host self-foreground.

#![cfg(windows)]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::Duration;

use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Threading::GetProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_MENU,
};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_FLAG_LOG_USAGE, SEE_MASK_NOCLOSEPROCESS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, ASFW_ANY, BringWindowToTop, EnumWindows, GetWindowThreadProcessId,
    IsWindowVisible, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOWNORMAL,
};

/// Launch `path` (a `.lnk`/`.cmd`/`.ps1`) and try to foreground its window.
pub fn launch_and_focus(path: &Path) -> Result<(), String> {
    // 1. Authorize the new process to take foreground (best-effort).
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }

    // 2. Build launch args by extension.
    let is_ps1 = path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("ps1"))
        .unwrap_or(false);
    let file_w: Vec<u16>;
    let params_w: Option<Vec<u16>>;
    if is_ps1 {
        file_w = wide("powershell.exe");
        let quoted = format!(
            "-NoProfile -ExecutionPolicy Bypass -File \"{}\"",
            path.to_string_lossy()
        );
        params_w = Some(wide(&quoted));
    } else {
        file_w = to_wide(path.as_os_str());
        params_w = None;
    }
    let verb_w = wide("open");
    let lp_params = match &params_w {
        Some(v) => PCWSTR(v.as_ptr()),
        None => PCWSTR::null(),
    };

    // 3. ShellExecuteExW → process handle.
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_LOG_USAGE,
        lpVerb: PCWSTR(verb_w.as_ptr()),
        lpFile: PCWSTR(file_w.as_ptr()),
        lpParameters: lp_params,
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info).map_err(|e| e.to_string())?; }

    // 4. PID; close our handle (we only need the PID for the window lookup).
    let hprocess = info.hProcess;
    if hprocess.is_invalid() {
        return Ok(()); // e.g. DDE re-activated an existing window
    }
    let pid = unsafe { GetProcessId(hprocess) };
    unsafe {
        let _ = CloseHandle(hprocess);
    }

    // 5. Best-effort foreground on a worker thread (don't block the UI thread).
    if pid != 0 {
        std::thread::spawn(move || focus_pid(pid));
    }
    Ok(())
}

/// Poll for the launched process's first visible top-level window and foreground it.
fn focus_pid(pid: u32) {
    for _ in 0..10 {
        let mut state = FindState {
            target_pid: pid,
            found: None,
        };
        let lparam = LPARAM(&mut state as *mut _ as isize);
        unsafe {
            let _ = EnumWindows(Some(enum_cb), lparam);
        }
        if let Some(hwnd) = state.found {
            unsafe {
                spoof_alt();
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
                let _ = BringWindowToTop(hwnd);
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // No window found (typical for console apps): the console host already
    // self-foregrounded courtesy of the ASFW grant. Nothing more to do.
}

#[repr(C)]
struct FindState {
    target_pid: u32,
    found: Option<HWND>,
}

unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let s = &mut *(lparam.0 as *mut FindState);
    let mut pid: u32 = 0;
    let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32));
    if pid == s.target_pid && IsWindowVisible(hwnd).as_bool() {
        s.found = Some(hwnd);
        return BOOL(0); // FALSE → stop enumerating
    }
    BOOL(1) // TRUE → keep going
}

/// Synthesize a harmless ALT press/release so the system treats the next
/// `SetForegroundWindow` as user-initiated (defeats the foreground lock).
unsafe fn spoof_alt() {
    let down = KEYBDINPUT {
        wVk: VK_MENU,
        wScan: 0,
        dwFlags: Default::default(),
        time: 0,
        dwExtraInfo: 0,
    };
    let mut input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0::default(),
    };
    input.Anonymous.ki = down;
    let mut release = input;
    release.Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;
    let inputs = [input, release];
    let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}

/// UTF-16 + NUL terminator.
fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn to_wide(os: &OsStr) -> Vec<u16> {
    os.encode_wide().chain(std::iter::once(0)).collect()
}
