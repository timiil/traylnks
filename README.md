# TrayLnks

**中文** | [English](#english)

TrayLnks 是一个轻量级 Windows 托盘链接启动器。它把一个本地文件夹映射成多级系统托盘菜单：文件系统就是配置。

适合把常用 `.lnk`、脚本、项目入口按目录整理起来，然后从托盘快速启动。

## 功能

- 文件夹递归映射为托盘子菜单
- 启动项映射为菜单项：`<name>.lnk`、`<name>.cmd`、`<name>.ps1`
- `.lnk` / `.cmd` 通过 Windows Shell `open` 动作启动
- `.ps1` 通过 `powershell.exe -NoProfile -ExecutionPolicy Bypass -File <path>` 启动，避免默认用编辑器打开
- 启动后尽量把目标窗口带到前台
- 同名图片可作为菜单图标：`<name>.ico`、`<name>.png`、`<name>.jpg`、`<name>.jpeg`
- 文件夹也支持同名图标，例如 `Development/` + `Development.png`
- item 没有同名图标时，会从 50 个内置 fallback 图标中按路径稳定分配一个
- `.station` 文件可按 Windows 主机名过滤某个目录树
- 其他普通文件会被忽略，不影响目录同步和菜单生成

完整产品规则见 [prd.md](./prd.md)。

## 使用示例

监听目录：

```text
TrayLauncher/
├── Development/
│   ├── Terminal.lnk
│   └── Codex.lnk
└── Projects/
    └── Snooker/
        └── Open Workspace.lnk
```

生成托盘菜单：

```text
Development >
    Terminal
    Codex

Projects >
    Snooker >
        Open Workspace
```

同名图标示例：

```text
Codex.lnk
Codex.png

Development/
Development.ico
```

## .station 主机过滤

在目录中放置 `.station` 文件，每行写一个允许显示该目录的 Windows 主机名：

```text
DESKTOP-PAMJPBD
TIM-LAPTOP
```

规则：

- 不区分大小写
- 精确匹配主机名
- 空行会被忽略
- `#` 开头的行视为注释
- 如果 `.station` 为空，则该目录在所有主机上都不显示
- 父目录不匹配时，整个子树不会进入菜单

## 运行

启动已构建的程序：

```powershell
Start-Process .\traylnks.exe
```

首次启动时还没有监听路径。打开 **Settings -> Paths -> Browse...**，选择一个本地文件夹并保存，托盘菜单会立即重建并开始监听文件变化。

配置文件由设置界面管理，位置：

```text
%APPDATA%\com.traylnks.app\config.toml
```

示例：

```toml
watch_path = "D:\\TrayLauncher"
cloud_path = "D:\\OneDrive\\TrayLauncherSync"
start_minimized = true
autostart = false
```

`cloud_path` 是可选同步路径；菜单始终以本地 `watch_path` 为运行源。

## 项目结构

```text
frontend/          静态设置界面，HTML/JS/CSS
src-tauri/         Tauri v2 + Rust 后端
  src/
    menu_tree.rs   扫描目录并生成菜单树
    tray.rs        构建托盘菜单、刷新、菜单点击路由
    watcher.rs     文件变化监听与防抖刷新
    icon.rs        同名图标解析与解码
    config.rs      TOML 配置读写
    station.rs     .station 主机过滤
    commands.rs    设置界面调用的 Tauri commands
tools/icon-gen/    应用图标源图与生成工具
```

## 构建

当前运行平台定位为 Windows 10 / 11。目前项目实际走通过、并在本文档中维护的构建路径是：

```text
WSL Debian -> cargo-xwin -> x86_64-pc-windows-msvc -> traylnks.exe
```

也就是说，下面展示的是在 WSL Debian 中交叉编译 Windows `.exe` 的流程。

一次性安装工具链：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
sudo apt install -y build-essential pkg-config libssl-dev nsis lld llvm clang
sudo apt install -y libayatana-appindicator3-dev
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
cargo install --locked tauri-cli --version "^2.0"
```

构建独立 `.exe`：

```bash
cd src-tauri
cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc -- --locked
```

输出文件：

```text
src-tauri/target/x86_64-pc-windows-msvc/release/traylnks.exe
```

当前 `tauri.conf.json` 的 `bundle.targets` 为 `[]`，因此默认只生成独立可执行文件，不生成安装包。需要 NSIS 安装包时，可把 `bundle.targets` 改为 `["nsis"]`。

## 图标

主图标源图：

```text
tools/icon-gen/assets/traylnks-icon-source.png
```

重新生成图标：

```bash
(cd tools/icon-gen && cargo run --release)
(cd src-tauri && cargo tauri icon ../tools/icon-gen/assets/traylnks-icon-source.png)
```

托盘图标使用 Tauri 的默认应用图标，因此重新生成 `src-tauri/icons/icon.ico` / `icon.png` 后，tray icon 和 exe 资源图标都会随之更新。fallback 菜单图标位于：

```text
src-tauri/assets/fallback-icons/
```

## 测试与检查

格式化：

```bash
cargo fmt --all --manifest-path src-tauri/Cargo.toml
cargo fmt --all --manifest-path tools/icon-gen/Cargo.toml
```

在 WSL 上检查 Windows 目标：

```bash
cd src-tauri
cargo xwin clippy --target x86_64-pc-windows-msvc -- -D warnings
```

单元测试主要覆盖 Windows 相关逻辑。在 WSL 中可先构建 Windows 测试二进制，再通过 Windows interop 运行：

```bash
cd src-tauri
cargo xwin test --target x86_64-pc-windows-msvc --lib --no-run
```

然后运行 `target/.../debug/deps/traylnks_lib-*.exe`。

## Public GitHub 状态

本仓库当前面向公开 GitHub 仓库整理：

- README 已移除个人部署路径，构建与运行说明改为通用描述
- 仓库内未发现需要写入 README 的密钥、令牌或私有服务配置
- 示例路径均为通用 Windows 示例路径
- 当前还没有 `LICENSE` 文件；在添加许可证前，外部用户不应默认拥有复制、修改或再分发授权
- 当前 GitHub 仓库暂无正式 Release；需要分发给普通用户时，建议先发布带版本号的 Release 和校验信息

## English

TrayLnks is a lightweight Windows tray link launcher. It maps a local folder into a multi-level system tray menu: the filesystem is the configuration.

It is useful for organizing common `.lnk` shortcuts, scripts, and project entry points in folders, then launching them quickly from the tray.

## Features

- Folders become recursive tray submenus
- Launchables become menu items: `<name>.lnk`, `<name>.cmd`, `<name>.ps1`
- `.lnk` / `.cmd` are launched via the Windows Shell `open` verb
- `.ps1` files are launched with `powershell.exe -NoProfile -ExecutionPolicy Bypass -File <path>` so they run instead of opening in an editor
- The launched window is brought to the foreground on a best-effort basis
- Same-name images can provide menu icons: `<name>.ico`, `<name>.png`, `<name>.jpg`, `<name>.jpeg`
- Folders support same-name icons, for example `Development/` + `Development.png`
- Items without same-name icons get a stable path-based assignment from 50 built-in fallback icons
- `.station` files can restrict a subtree to specific Windows hostnames
- Other files are ignored and do not affect sync or menu generation

See [prd.md](./prd.md) for the full product rules.

## Example

Watched folder:

```text
TrayLauncher/
├── Development/
│   ├── Terminal.lnk
│   └── Codex.lnk
└── Projects/
    └── Snooker/
        └── Open Workspace.lnk
```

Generated tray menu:

```text
Development >
    Terminal
    Codex

Projects >
    Snooker >
        Open Workspace
```

Same-name icon example:

```text
Codex.lnk
Codex.png

Development/
Development.ico
```

## .station Host Filtering

Place a `.station` file in a folder and list the Windows hostnames allowed to see that folder:

```text
DESKTOP-PAMJPBD
TIM-LAPTOP
```

Rules:

- Case-insensitive
- Exact hostname match
- Empty lines are ignored
- Lines beginning with `#` are comments
- An empty `.station` file hides the folder on every host
- If a parent folder does not match, the whole subtree is skipped

## Running

Start a built executable:

```powershell
Start-Process .\traylnks.exe
```

On first launch, no watch path is configured. Open **Settings -> Paths -> Browse...**, choose a local folder, and save. The tray menu rebuilds immediately and starts watching for changes.

The settings UI manages this TOML config file:

```text
%APPDATA%\com.traylnks.app\config.toml
```

Example:

```toml
watch_path = "D:\\TrayLauncher"
cloud_path = "D:\\OneDrive\\TrayLauncherSync"
start_minimized = true
autostart = false
```

`cloud_path` is optional. The tray menu always runs from the local `watch_path`.

## Repository Layout

```text
frontend/          Static Settings UI, HTML/JS/CSS
src-tauri/         Tauri v2 + Rust backend
  src/
    menu_tree.rs   Scans folders and builds the menu tree
    tray.rs        Builds the tray menu, refreshes it, routes menu clicks
    watcher.rs     Watches filesystem changes with debounce
    icon.rs        Resolves and decodes same-name icons
    config.rs      Reads and writes TOML config
    station.rs     Implements .station hostname filtering
    commands.rs    Tauri commands used by the Settings UI
tools/icon-gen/    App icon source artwork and generation helper
```

## Build

The runtime target is Windows 10 / 11. The only build path currently exercised and maintained in this README is:

```text
WSL Debian -> cargo-xwin -> x86_64-pc-windows-msvc -> traylnks.exe
```

In other words, the commands below cross-compile a Windows `.exe` from WSL Debian.

One-time setup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
sudo apt install -y build-essential pkg-config libssl-dev nsis lld llvm clang
sudo apt install -y libayatana-appindicator3-dev
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
cargo install --locked tauri-cli --version "^2.0"
```

Build a standalone `.exe`:

```bash
cd src-tauri
cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc -- --locked
```

Output:

```text
src-tauri/target/x86_64-pc-windows-msvc/release/traylnks.exe
```

`bundle.targets` is currently `[]` in `tauri.conf.json`, so the default build produces a standalone executable only, not an installer. To build an NSIS installer, set `bundle.targets` to `["nsis"]`.

## Icons

Source artwork:

```text
tools/icon-gen/assets/traylnks-icon-source.png
```

Regenerate icons:

```bash
(cd tools/icon-gen && cargo run --release)
(cd src-tauri && cargo tauri icon ../tools/icon-gen/assets/traylnks-icon-source.png)
```

The tray icon uses Tauri's default application icon, so regenerating `src-tauri/icons/icon.ico` / `icon.png` updates both the tray icon and the exe resource icon. Fallback menu icons live at:

```text
src-tauri/assets/fallback-icons/
```

## Tests And Checks

Format:

```bash
cargo fmt --all --manifest-path src-tauri/Cargo.toml
cargo fmt --all --manifest-path tools/icon-gen/Cargo.toml
```

Check the Windows target from WSL:

```bash
cd src-tauri
cargo xwin clippy --target x86_64-pc-windows-msvc -- -D warnings
```

Most unit tests cover Windows-specific behavior. From WSL, build the Windows test binary and run it through Windows interop:

```bash
cd src-tauri
cargo xwin test --target x86_64-pc-windows-msvc --lib --no-run
```

Then run `target/.../debug/deps/traylnks_lib-*.exe`.

## Public GitHub Status

This README is prepared for a public GitHub repository:

- Personal deployment paths have been removed from the public instructions
- No README-worthy secrets, tokens, or private service configuration were found in the repository
- Example paths are generic Windows examples
- There is currently no `LICENSE` file; until one is added, external users should not assume rights to copy, modify, or redistribute the project
- There is currently no official GitHub Release; for end-user distribution, publish a versioned Release with checksums
