# TrayLnks

**中文** | [English](#english)

轻量级 Windows 托盘链接启动器：把一个本地文件夹映射成多级系统托盘菜单，**文件系统即配置**。

<p align="center">
  <img src="docs/screenshot.png" alt="TrayLnks 托盘菜单" width="360">
</p>

## 功能

- 文件夹 → 递归子菜单
- `<name>.lnk` / `.cmd` / `.ps1` → 菜单项（`.ps1` 用 `powershell -NoProfile -ExecutionPolicy Bypass -File` 启动，避免被编辑器打开）
- 启动后尽量把窗口带到前台
- 同名图片作为图标：`<name>.ico/.png/.jpg/.jpeg`（文件夹也支持，如 `Development/` + `Development.png`）
- 无同名图标时，按路径哈希从 50 个内置 fallback 图标中稳定分配一个
- **终端会话监控**：监控 WSL／直接 SSH 中已接入的 Codex、Claude Code，通过 tab 标题字符提示回复结束、等待输入、静默或明确异常；扫描频率可配，支持已阅、忽略本轮和暂停。[使用说明](docs/terminal-monitor.md)
- **TakeSnap / ViewSnaps**：保存所有 Windows Terminal 窗口中每个 tab 的当前视区截图和文字，重启后可按时间回看
- `.station` 文件按 Windows 主机名过滤子树
- 其他文件被忽略

完整规则见 [prd.md](./prd.md)。

## 示例

监听目录：

```text
TrayLauncher/
├── Development/
│   ├── Terminal.lnk
│   └── Codex.lnk        # 配 Codex.png 作为图标
└── Projects/
    └── Snooker/
        └── Open Workspace.lnk
```

生成菜单：

```text
Development >
    Terminal
    Codex
Projects >
    Snooker >
        Open Workspace
```

## .station 主机过滤

在目录中放 `.station`，每行一个允许显示该目录的主机名：

```text
DESKTOP-PAMJPBD
TIM-LAPTOP
```

不区分大小写、精确匹配；`#` 起始为注释，空行忽略；空 `.station` 在所有主机隐藏该目录；父目录不匹配则整个子树跳过。

## 终端会话监控

同时运行多个 AI CLI 时，TrayLnks 会在需要关注的 **tab 标题**上显示状态，方便你找到等待下一步的会话。明确的回复结束、待输入信号来自工具的生命周期 hooks；输出静默作为兜底，避免仅凭“不刷屏了”就判定任务完成或断线。监控在后台进行，不切换 tab、不抢焦点、不向终端输入命令。

| 标题标记 | 含义 |
| --- | --- |
| `✓` | 本轮回复结束，等待下一步；不保证整项任务已成功 |
| `?` | 明确等待用户输入、授权或确认 |
| `~` | 超过静默阈值未见有效输出，可能仍在计算或等待网络；完成／断链均未确认 |
| `!` | 明确的终止错误或 SSH 异常退出，可在会话列表查看原因 |

提醒时，状态符与 `·` 每 500 毫秒交替，默认持续 6 秒，再留下静态标记。托盘可辅助短暂变色 1 秒，至少间隔 60 秒；多个会话同时结束时合并提示，冷却期内不排队补闪。

在 **Settings → Monitoring** 调整监控开关和以下选项，按 **Apply** 保存，**Cancel** 撤销未保存的设置：

| 配置项 | 默认值 | 范围／行为 |
| --- | --- | --- |
| Scan interval（扫描间隔） | 10 秒 | 1–3600 秒，控制静默检查频率 |
| Quiet threshold（静默阈值） | 20 秒 | 1–86400 秒，控制多久没有有效输出才提示 |
| Tab header flash（标题闪动时长） | 6 秒 | 0–300 秒，0 表示仅保留静态标记 |
| Also pulse the tray icon briefly（托盘辅助提示） | 开启 | 可关闭；开启时固定短闪 1 秒、至少间隔 60 秒 |

静默提示最多会额外等待一个扫描间隔；明确的 hook 状态即时处理，不等待下一次扫描。

**首次接入：**

1. 打开 **Settings → Monitoring → Connect WSL and SSH sessions**，点击 **Prepare setup commands**。此操作导出 exe 内嵌的桥接脚本并生成命令，不自动安装 hooks。
2. 在 WSL 运行所用工具的 `install-hooks --tool codex` 或 `--tool claude` 命令，按工具要求审核／信任 hooks。安装会合并已有配置、备份原文件，重复执行不会重复添加相同命令。
3. 在各 tab 运行生成的 `shell --state-dir ...` 命令，再在该监控 shell 中启动或重启 Codex／Claude Code。已接入的 AI 会话自动监控；普通已接入 shell 默认关闭，可在列表点击 **Enable tab** 开启。
4. 使用 SSH 时，将生成命令中的 `user@host` 替换为实际目标，按命令复制脚本、在远端安装 hooks，再通过生成的 `ssh --state-dir ... -- user@host` 入口连接。连接本身也受监控，包括 AI CLI 启动前的连接失败。

WSL 和远端 Linux 均需 Python 3.8+，工具版本需支持对应 hooks。原先直接启动的 CLI 不会自动接入，需要从监控入口重新启动；SSH 需要重新连接。

**减少重复打扰：**同一轮最多主动提醒一次，后续状态升级只更新静态标记；下一轮请求开始后重新布防。托盘会话菜单或 Monitoring 页可执行以下操作，会话操作即时生效：

- **Mark read**：清除当前标记，本轮不再闪动；后续状态升级仍可显示新的静态标记。
- **Ignore turn**：忽略本轮全部后续标记，下一轮自动恢复。
- **Pause tab / Resume**：持续暂停该 tab，直到手动恢复；恢复时不补闪旧事件。TrayLnks 重启也不补闪历史事件。

当前支持 Windows Terminal 内的 WSL 本地会话，以及从 WSL 发起的直接 SSH Linux 会话，每个 tab 对应一个监控 shell；原生 PowerShell、tmux/screen、分屏聚合不在首版范围。手动固定 tab 标题或 `suppressApplicationTitle` 可能遮住标题标记。观察端心跳失效会显示 **Observer unavailable**，与确定断链分开呈现。

监控数据保存在 `%APPDATA%\com.traylnks.launcher\monitor\`，仅保存会话身份、标题、主机、状态、活动计数和提醒策略，不保存终端正文或用户输入，不参与云同步。远端状态通过现有 SSH 连接回传，不额外开放端口。完整接入、故障处理及验证命令见 [终端监控说明](docs/terminal-monitor.md)。

## 终端快照

**TakeSnap** 为所有 Windows Terminal 窗口的 tab 保存当前视区截图、完整 tab 标题和可复制文字，包括手动打开的终端。即使 tab 停在历史输出中间，也会记录该位置；分屏保留在同一张图中，文字按分屏分别记录。适合关机前保存工作现场，重启后通过 **ViewSnaps** 回看终端的开启情况。

1. **保存现场**：在托盘菜单或 ViewSnaps 窗口点击 **TakeSnap**。采集期间会短暂切换窗口和 tab，完成后恢复原选中 tab、窗口状态及焦点；不会输入命令、主动滚动或修改剪贴板。等待采集结束后再继续操作终端，托盘提示和查看窗口均会显示进度及结果。
2. **选择记录**：托盘 → **ViewSnaps** 打开独立窗口。Table 默认按时间从新到旧列出快照编码、时间、窗口数、tab 数和采集结果；点击一条记录，再从左侧按窗口分组的 tab 清单选择明细。重启后无需先打开原来的终端，即可查看已有快照。
3. **查看明细**：切换 **截图**、**视区文字**、**记录文本**。截图支持适应窗口、原始尺寸及缩放；视区文字可选择、复制；记录文本汇总整条快照的窗口、tab 和错误信息。可点击 **刷新** 重新读取本地记录。

查看完毕后，点击右上角 **关闭** 或按 **Esc** 返回托盘；也可使用系统标题栏的关闭按钮。关闭查看窗口不会退出 TrayLnks 或中断正在进行的采集，之后可从托盘再次打开 **ViewSnaps**。

<p align="center">
  <img src="docs/snaps-overview.png" alt="ViewSnaps 快照表格、Tab 清单与截图预览，敏感信息已打码" width="960">
</p>

<p align="center"><sub>快照列表与截图预览。私人项目标题、主机名及终端正文已打码。</sub></p>

<p align="center">
  <img src="docs/snaps-text.png" alt="ViewSnaps 视区文字与复制文本功能，敏感信息已打码" width="960">
</p>

<p align="center"><sub>切换到视区文字，查看各分屏的可复制内容。终端正文已打码。</sub></p>

- **本地保存**：位于 `%APPDATA%\com.traylnks.launcher\snaps\`，每次一个目录，编码为本地时间 `yyyyMMdd-HHmm`；同一分钟重复采集追加 `-02`、`-03`，不会覆盖。目录包含 `manifest.json`、`record.txt` 和各 tab 的 PNG／TXT；仅手动采集、长期保留，不参与启动器目录的云同步。
- **部分失败**：窗口或 tab 不可访问、关闭或超时，会保留其他已采集内容和失败原因。截图和文字独立记录结果，单项失败不影响查看其余内容；没有发现终端时不生成空记录。
- **平台与边界**：仅支持 Windows Terminal，要求 Windows 10 1903+／Windows 11；不会自动提权访问管理员终端。快照用于回看，不自动重建终端或恢复命令。各 tab 依次采集，持续输出的画面和文字可能存在短暂时间差。

## 网盘同步（v0.2，阿里云盘）

设置 → Cloud 标签页：用阿里云盘 App 扫码授权，选择云端文件夹（如 `/TrayLnks`），即可在多台机器间双向镜像 `watch_path`。冲突按修改时间「最后写入胜出」，删除会双向传播。刷新间隔默认 30 分钟（可选 15/60/120）。

<p align="center">
  <img src="docs/screenshot1.png" alt="TrayLnks Cloud 同步设置（阿里云盘扫码授权）" width="480">
</p>

- **零注册**：走网页客户端公开扫码登录（[aligo](https://github.com/foyoux/aligo) 同款流程），无需在阿里云盘开放平台注册应用、无需 AppKey。
- 架构按 `CloudProvider` trait 设计，当前仅实现阿里云盘，后续可扩展其他供应商。
- 凭据存入系统凭据管理器（Windows Credential Manager / DPAPI），**不会**写入 `config.toml`（该文件会序列化到前端）。
- 已知限制：`.lnk` 内嵌机器相关绝对路径，跨机同步后目标可能无法解析；`.cmd`/`.ps1` 可移植性更好。

## 运行

```powershell
Start-Process .\traylnks.exe
```

首次启动后在 **Settings → Paths → Browse...** 选择监听目录并保存，菜单立即重建并开始监听。配置由设置界面管理：

```text
%APPDATA%\com.traylnks.launcher\config.toml
```

```toml
watch_path              = "D:\\TrayLauncher"
start_minimized         = true
autostart               = false
# 网盘同步（v0.2，可选）
cloud_enabled           = false
cloud_provider          = "aliyun"
cloud_folder            = "/TrayLnks"     # 阿里云盘里的同步根目录，首次同步时不存在则创建
cloud_sync_interval_min = 30              # 自动同步间隔（分钟）

[monitor]
enabled                 = true
scan_interval_secs      = 10              # 静默检查频率（秒）
quiet_threshold_secs    = 20              # 无有效输出的提示阈值（秒）
tab_flash_duration_secs = 6               # 标题字符闪动时长（秒）；0 为静态标记
tray_flash_enabled      = true            # 托盘辅助短提示
```

## 项目结构

```text
frontend/          静态设置界面（HTML/JS/CSS）
src-tauri/         Tauri v2 + Rust 后端
  src/
    menu_tree.rs   扫描目录生成菜单树
    tray.rs        托盘菜单构建 / 刷新 / 点击路由
    watcher.rs     文件变化监听 + 防抖
    icon.rs        同名图标解析 + fallback
    config.rs      TOML 配置读写
    station.rs     .station 主机过滤
    commands.rs    设置界面调用的 commands
    monitor/       会话状态机、静默检查、提醒策略与接入命令
tools/icon-gen/    应用图标源图与生成工具
tools/terminal-agent/  WSL／SSH PTY 桥接脚本、CLI hooks 与集成测试
tools/ui-tests/    设置界面浏览器交互测试
```

## 构建

目标 Windows 10/11，从 WSL Debian 交叉编译（`cargo-xwin` → `x86_64-pc-windows-msvc`）。

一次性工具链：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && source "$HOME/.cargo/env"
sudo apt install -y build-essential pkg-config libssl-dev nsis lld llvm clang libayatana-appindicator3-dev
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
cargo install --locked tauri-cli --version "^2.0"
```

构建独立 `.exe`：

```bash
cd src-tauri
cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc -- --locked
# → src-tauri/target/x86_64-pc-windows-msvc/release/traylnks.exe
```

`bundle.targets` 默认为 `[]`（仅独立 `.exe`）；改为 `["nsis"]` 可输出安装包。

## 图标

```bash
(cd tools/icon-gen && cargo run --release)   # 源图 → src-tauri/icons/icon.png
(cd src-tauri && cargo tauri icon ../tools/icon-gen/assets/traylnks-icon-source.png)
```

托盘图标复用 Tauri 默认应用图标，重新生成 `icons/icon.ico`/`icon.png` 后托盘与 exe 图标同步更新。fallback 菜单图标位于 `src-tauri/assets/fallback-icons/`。

## 测试

```bash
cargo fmt --all --manifest-path src-tauri/Cargo.toml
cd src-tauri && cargo xwin clippy --target x86_64-pc-windows-msvc -- -D warnings
cargo xwin test --target x86_64-pc-windows-msvc --lib --no-run   # 再运行 target/.../traylnks_lib-*.exe
```

---

## English

A lightweight Windows tray link launcher: map a local folder into a multi-level system tray menu — **the filesystem is the config**.

<p align="center">
  <img src="docs/screenshot.png" alt="TrayLnks tray menu" width="360">
</p>

## Features

- Folders → recursive submenus
- `<name>.lnk` / `.cmd` / `.ps1` → menu items (`.ps1` runs via `powershell -NoProfile -ExecutionPolicy Bypass -File`, so it executes instead of opening in an editor)
- Best-effort foreground focus on launch
- Same-name image as icon: `<name>.ico/.png/.jpg/.jpeg` (folders too, e.g. `Development/` + `Development.png`)
- Items without a matching icon get a stable path-hashed pick from 50 built-in fallbacks
- **Terminal monitoring** watches connected Codex / Claude Code sessions in WSL and direct SSH. Tab-title characters distinguish finished replies, input requests, quiet sessions, and explicit errors, with configurable scans and read / ignore / pause controls. See [setup and behavior](docs/terminal-monitor.md).
- **TakeSnap / ViewSnaps** save every Windows Terminal tab's current viewport image and text for viewing after a restart
- `.station` files filter a subtree by Windows hostname
- Other files are ignored

Full rules in [prd.md](./prd.md).

## Example

Watched folder:

```text
TrayLauncher/
├── Development/
│   ├── Terminal.lnk
│   └── Codex.lnk        # pair with Codex.png for an icon
└── Projects/
    └── Snooker/
        └── Open Workspace.lnk
```

Generated menu:

```text
Development >
    Terminal
    Codex
Projects >
    Snooker >
        Open Workspace
```

## .station Host Filtering

Drop a `.station` file in a folder, one allowed hostname per line:

```text
DESKTOP-PAMJPBD
TIM-LAPTOP
```

Case-insensitive, exact match; `#` lines are comments, blank lines ignored; an empty `.station` hides the folder everywhere; if a parent doesn't match, the whole subtree is skipped.

## Terminal Monitoring

When several AI CLI sessions run at once, TrayLnks marks the **tab title** of sessions that may need attention. Tool lifecycle hooks report finished replies and input requests; quiet-output detection provides a fallback. A lack of output alone does not confirm completion or a lost connection. Monitoring runs in the background without switching tabs, taking focus, or sending terminal input.

| Title marker | Meaning |
| --- | --- |
| `✓` | This reply has finished and awaits a next step; overall task success is not guaranteed |
| `?` | Explicitly waiting for input, permission, or confirmation |
| `~` | No effective output beyond the quiet threshold; computation or network waits may still be in progress, so completion / disconnection is unconfirmed |
| `!` | An explicit terminal error or abnormal SSH exit; the session list shows the reason |

The marker alternates with `·` every 500 ms for 6 seconds by default, then remains static. An optional tray accent lasts 1 second, with at least 60 seconds between pulses. Concurrent alerts share a pulse; alerts during the cooldown do not queue more pulses.

Open **Settings → Monitoring** to enable / disable monitoring and adjust these settings. **Apply** saves them; **Cancel** discards unsaved changes.

| Setting | Default | Range / behavior |
| --- | --- | --- |
| Scan interval | 10 seconds | 1–3600 seconds; how often to check for quiet sessions |
| Quiet threshold | 20 seconds | 1–86400 seconds without effective output before attention is needed |
| Tab header flash | 6 seconds | 0–300 seconds; 0 leaves a static marker only |
| Also pulse the tray icon briefly | On | Optional; fixed 1-second pulse, at least 60 seconds apart |

Quiet detection can take up to one additional scan interval. Explicit hook events are processed immediately, without waiting for a scan.

**Connect sessions:**

1. Open **Settings → Monitoring → Connect WSL and SSH sessions**, then click **Prepare setup commands**. This exports the bridge embedded in the exe and generates commands; it does not install hooks automatically.
2. In WSL, run the generated `install-hooks --tool codex` or `--tool claude` command for the tools you use, then review / trust hooks as required by each tool. Installation merges existing settings, backs up the original file, and avoids duplicate commands on repeated runs.
3. Run the generated `shell --state-dir ...` command in each tab, then start or restart Codex / Claude Code inside that monitored shell. Connected AI sessions are monitored automatically. Ordinary connected shells remain unmonitored until you click **Enable tab**.
4. For SSH, replace `user@host` in the generated commands, copy the bridge, install hooks on the remote host, and connect through the generated `ssh --state-dir ... -- user@host` entry. The connection itself is monitored, including failures before an AI CLI starts.

WSL and remote Linux hosts require Python 3.8+, and each CLI must support the relevant hooks. Existing CLI processes need to restart through the monitored entry; SSH sessions need to reconnect.

**Control reminders:** each turn gets at most one active alert. Later state changes update the static marker; the next request rearms monitoring. Session actions in the tray menu or Monitoring page take effect immediately:

- **Mark read** clears the current marker without flashing again this turn; later state changes may show a new static marker.
- **Ignore turn** suppresses all further markers this turn and resets on the next request.
- **Pause tab / Resume** keeps a tab paused until manually resumed, without replaying old alerts. Restarting TrayLnks also does not replay historical alerts.

This release supports local WSL sessions and direct SSH Linux sessions launched from WSL in Windows Terminal, with one monitored shell per tab. Native PowerShell, tmux/screen, and split-pane aggregation are outside this release's scope. Fixed tab titles or `suppressApplicationTitle` may hide title markers. An expired observer heartbeat shows **Observer unavailable**, separately from a confirmed connection failure.

Monitoring stores session identity, title, host, state, activity counters, and reminder policies under `%APPDATA%\com.traylnks.launcher\monitor\`. It does not retain terminal contents or user input, and does not participate in cloud sync. Remote status travels over the existing SSH connection without an extra listening port. See the [terminal monitoring guide](docs/terminal-monitor.md) (Chinese) for full setup, troubleshooting, and verification commands.

## Terminal Snapshots

**TakeSnap** saves the current viewport image, full tab title, and copyable text for every tab in all Windows Terminal windows, including terminals opened outside TrayLnks. A tab scrolled to older output is captured at that position. Split panes share one tab screenshot, with visible text recorded separately for each pane. Save your workspace before shutdown, then use **ViewSnaps** to review it after restarting.

1. **Capture**: click **TakeSnap** in the tray menu or ViewSnaps window. Capture briefly switches windows and tabs, then restores their selected tabs, window states, and focus. It sends no terminal input, does not scroll, and leaves the clipboard untouched. Wait for capture to finish before using the terminals; progress and results appear in the tray tooltip and viewer.
2. **Select**: open **ViewSnaps** from the tray. Its table lists snapshots newest first, with their IDs, times, window/tab counts, and results. Select a record, then a tab grouped under its window in the sidebar. Previously captured terminals do not need to be running to view their snapshots.
3. **Inspect**: switch between **截图** (screenshot), **视区文字** (visible text), and **记录文本** (record text). Images support fit-to-window, original size, and zoom. Visible text can be selected or copied; record text combines all windows, tabs, and errors. **刷新** refreshes the local list.

Click **关闭** (close) at the top right, press **Esc**, or use the native title-bar close button to return to the tray. Closing the viewer keeps TrayLnks and any active capture running; reopen **ViewSnaps** from the tray whenever needed.

<p align="center">
  <img src="docs/snaps-overview.png" alt="ViewSnaps snapshot table, tab list and screenshot preview with private information redacted" width="960">
</p>

<p align="center">
  <img src="docs/snaps-text.png" alt="ViewSnaps visible text and copy control with private information redacted" width="960">
</p>

The screenshots above redact private project titles, the hostname, and terminal output.

- **Storage**: `%APPDATA%\com.traylnks.launcher\snaps\`, one directory per capture, named in local time as `yyyyMMdd-HHmm`. Captures in the same minute add `-02`, `-03`, etc. Each contains `manifest.json`, `record.txt`, and per-tab PNG/TXT files. Capture is manual, records are retained, and snapshots do not participate in launcher-folder cloud sync.
- **Partial failures**: closed, inaccessible, or timed-out windows/tabs retain an error alongside successfully captured content. Images and text have independent results. No empty record is created when no terminals are found.
- **Requirements**: Windows Terminal on Windows 10 1903+ or Windows 11. No automatic elevation, terminal reconstruction, or command restoration. Tabs are captured sequentially, so continuously updating image and text content may differ slightly in time.

## Cloud Sync (v0.2, Aliyun Drive)

Settings → Cloud tab: authorize by scanning a QR with the Aliyun Drive app, pick a cloud folder (e.g. `/TrayLnks`), and `watch_path` is two-way mirrored across machines. Conflicts resolve last-write-wins by mtime; deletions propagate both ways. Refresh interval defaults to 30 min (15/60/120 selectable).

<p align="center">
  <img src="docs/screenshot1.png" alt="TrayLnks Cloud sync settings (Aliyun Drive QR auth)" width="480">
</p>

- **Zero registration**: uses the web-client public QR login (the same flow as [aligo](https://github.com/foyoux/aligo)) — no app registration or AppKey needed.
- Built around a `CloudProvider` trait — only Aliyun Drive is implemented today; other providers are additive later.
- Tokens live in the OS credential store (Windows Credential Manager / DPAPI) and **never** in `config.toml` (which is serialized to the webview).
- Known limitation: `.lnk` files embed machine-specific absolute target paths and may not resolve on another machine after sync; `.cmd`/`.ps1` are more portable.

## Running

```powershell
Start-Process .\traylnks.exe
```

On first launch, pick a folder via **Settings → Paths → Browse...** and save — the menu rebuilds and starts watching. Config is managed by the settings UI:

```text
%APPDATA%\com.traylnks.launcher\config.toml
```

```toml
watch_path              = "D:\\TrayLauncher"
start_minimized         = true
autostart               = false
# Cloud sync (v0.2, optional)
cloud_enabled           = false
cloud_provider          = "aliyun"
cloud_folder            = "/TrayLnks"     # sync root inside the cloud drive; created on first sync if missing
cloud_sync_interval_min = 30              # auto-sync cadence in minutes

[monitor]
enabled                 = true
scan_interval_secs      = 10              # quiet-check cadence in seconds
quiet_threshold_secs    = 20              # seconds without effective output before attention
tab_flash_duration_secs = 6               # title character animation in seconds; 0 for static only
tray_flash_enabled      = true            # brief auxiliary tray pulse
```

## Repository Layout

```text
frontend/          Static settings UI (HTML/JS/CSS)
src-tauri/         Tauri v2 + Rust backend
  src/
    menu_tree.rs   Scans folders into a menu tree
    tray.rs        Tray menu build / refresh / click routing
    watcher.rs     Filesystem watch + debounce
    icon.rs        Same-name icon resolution + fallbacks
    config.rs      TOML config read/write
    station.rs     .station hostname filtering
    commands.rs    Commands used by the settings UI
    monitor/       Session state, quiet checks, reminder policies, and setup commands
tools/icon-gen/    App icon source artwork + generator
tools/terminal-agent/  WSL / SSH PTY bridge, CLI hooks, and integration tests
tools/ui-tests/    Browser interaction tests for settings
```

## Build

Targeting Windows 10/11, cross-compiled from WSL Debian (`cargo-xwin` → `x86_64-pc-windows-msvc`).

One-time toolchain:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && source "$HOME/.cargo/env"
sudo apt install -y build-essential pkg-config libssl-dev nsis lld llvm clang libayatana-appindicator3-dev
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
cargo install --locked tauri-cli --version "^2.0"
```

Build a standalone `.exe`:

```bash
cd src-tauri
cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc -- --locked
# → src-tauri/target/x86_64-pc-windows-msvc/release/traylnks.exe
```

`bundle.targets` defaults to `[]` (standalone `.exe` only); set it to `["nsis"]` for an installer.

## Icons

```bash
(cd tools/icon-gen && cargo run --release)   # source → src-tauri/icons/icon.png
(cd src-tauri && cargo tauri icon ../tools/icon-gen/assets/traylnks-icon-source.png)
```

The tray icon reuses Tauri's default app icon, so regenerating `icons/icon.ico`/`icon.png` updates both tray and exe. Fallback menu icons live in `src-tauri/assets/fallback-icons/`.

## Tests

```bash
cargo fmt --all --manifest-path src-tauri/Cargo.toml
cd src-tauri && cargo xwin clippy --target x86_64-pc-windows-msvc -- -D warnings
cargo xwin test --target x86_64-pc-windows-msvc --lib --no-run   # then run target/.../traylnks_lib-*.exe
```
