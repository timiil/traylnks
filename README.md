# TrayLnks

A lightweight **Windows tray link launcher**. The filesystem is the config: a
watched local folder becomes a multi-level tray menu.

- folder → submenu (recursive)
- launchables → menu items: `<name>.lnk`, `<name>.cmd`, `<name>.ps1`
  - `.lnk`/`.cmd`: run via the shell `open` verb; `.ps1` is invoked as
    `powershell.exe -NoProfile -ExecutionPolicy Bypass -File <path>` (its default
    association would otherwise open an editor)
  - the launched window is brought to the **foreground** (`AllowSetForegroundWindow`
    + `ShellExecuteExW` + best-effort `SetForegroundWindow`)
- same-name image (`<name>.ico/.png/.jpg/.jpeg`) → menu icon (folder icons too)
- `.station` file → restricts a subtree to specific Windows hostnames
- other files (`.md`, `.txt`, `.bat`, …) are ignored (whitelist rendering)

See [`prd.md`](./prd.md) for the full spec. v0.1 scope is defined in PRD §13.

## Layout

```
frontend/          static Settings UI (HTML/JS/CSS) shown in a hidden WebView window
src-tauri/         Rust backend (Tauri v2)
  src/
    menu_tree.rs   recursive scan → MenuNode tree (whitelist + .station + icons)
    tray.rs        tree → native menu; rebuild; click routing
    watcher.rs     notify-debouncer (500ms) → rebuild
    icon.rs        icon priority resolution + decode
    config.rs      TOML config load/save
    station.rs     .station host filtering
    commands.rs    #[tauri::command]s for the Settings UI
tools/icon-gen/    generates the source app icon PNG
```

## Build (cross-compile from WSL Debian → Windows)

One-time toolchain setup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && source "$HOME/.cargo/env"
sudo apt install -y build-essential pkg-config libssl-dev nsis lld llvm clang
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
cargo install --locked tauri-cli --version "^2.0"
```

> The `tray-icon` feature makes the Tauri bundler probe for an appindicator
> library on the Linux host even for a Windows cross-target. Install it too or
> the NSIS step panics with *"Can't detect any appindicator library"*:
> `sudo apt install -y libayatana-appindicator3-dev`

Regenerate icons (only when the design changes):

```bash
(cd tools/icon-gen && cargo run --release)        # -> src-tauri/icons/icon.png
(cd src-tauri && cargo tauri icon ../tools/.../icon.png)   # full icon set
```

Build the app (`bundle.targets` is `[]` — produces a standalone `.exe` only, no installer):

```bash
cd src-tauri
cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --locked
```

Output: `target/x86_64-pc-windows-msvc/release/traylnks.exe` — standalone binary
(WebView2 loader statically linked; relies on the OS-installed WebView2 runtime,
present on Windows 10/11). Re-enable an NSIS installer by setting `bundle.targets`
to `["nsis"]` in `tauri.conf.json`.

### Deploy the binary

Copy the built `.exe` to `D:\green\traylnks\` (the agreed output location):

```bash
cp target/x86_64-pc-windows-msvc/release/traylnks.exe /mnt/d/green/traylnks/
```

> **Gotcha:** if `traylnks.exe` is currently running from that path, Windows
> locks the file and the copy fails with `Input/output error`. Kill it first:
> `taskkill.exe /IM traylnks.exe /F` (or close it via the tray → Exit), then copy.

### Tests

Unit tests (`station.rs`, `menu_tree.rs`) target Windows, so `cargo test` can't
execute them on Linux. On WSL you can run them via the Windows interop: build
with `cargo xwin test --target x86_64-pc-windows-msvc --lib --no-run`, then run
the produced `target/.../debug/deps/traylnks_lib-*.exe` directly — WSL executes
it on real Windows. Clippy gate: `cargo xwin clippy --target x86_64-pc-windows-msvc -- -D warnings`.

## Run on Windows

```powershell
Start-Process .\traylnks.exe
```

On first launch there is no watch path yet: open **Settings → Paths → Browse…**,
pick a folder, and Save. The tray menu rebuilds and the file watcher starts.

## Config

Stored as TOML at `%APPDATA%\com.traylnks.app\config.toml` (managed by the
Settings UI):

```toml
watch_path = "D:\\TrayLauncher"
cloud_path = "D:\\OneDrive\\TrayLauncherSync"
start_minimized = true
autostart = false
```
