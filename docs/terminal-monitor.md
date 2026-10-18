# 终端会话监控

TrayLnks 监控通过桥接入口接入的 Windows Terminal tab。首版支持 WSL 本地和从 WSL 建立的直接 SSH Linux 会话，优先接入 Codex、Claude Code。每个 tab 对应一个受监管 shell；原生 PowerShell、tmux/screen、分屏聚合不在首版范围。

## 接入

1. 打开 Settings → Monitoring，展开 **Connect WSL and SSH sessions**，点击 **Prepare setup commands**。桥接脚本内嵌在 exe 中，此操作将脚本导出到应用自己的 monitor 目录，不自动修改终端或 AI 工具配置。
2. 在 WSL 运行生成的 `install-hooks --tool codex` 或 `--tool claude` 命令，仅安装实际使用的工具。安装会合并既有 hooks、保留原有设置，并为已有文件留下 `.traylnks-backup` 备份；重复运行不会重复添加相同命令。
3. 按工具要求审核／信任 hooks，然后在对应 tab 运行生成的 `shell --state-dir ...` 命令。在这个受监管 shell 中启动或重启 Codex / Claude Code。
4. SSH：把生成命令中的 `user@host` 替换为实际目标，创建远端目录、复制脚本，并在远端安装相应 hooks。之后使用生成的 `ssh --state-dir ... -- user@host` 入口连接。SSH 选项放在目标主机之前，例如 `ssh --state-dir ... -- -p 2222 -J jump user@host`。认证、密码输入和窗口缩放沿用 SSH 自身行为。

桥接脚本使用 Python 3.8+ 标准库，WSL 和远端均需要 Python 3。Codex 安装路径遵循 `CODEX_HOME`，默认 `~/.codex/hooks.json`；Claude Code 遵循 `CLAUDE_CONFIG_DIR`，默认 `~/.claude/settings.json`。使用自定义工具环境时，在相同环境中执行安装命令。CLI 版本必须支持对应生命周期 hooks；可用信号以工具实际支持情况为准。[Codex hooks](https://developers.openai.com/codex/hooks)、[Claude Code hooks](https://code.claude.com/docs/en/hooks)

直接运行未接入的旧 CLI 不会自动得到后台监控。可在现有 tab 内进入监控 shell；SSH 需要通过新入口重新连接。普通已接入 shell 在列表中出现，但默认不监控，点击 **Enable tab** 手动布防。通过 SSH 监控入口发起的连接本身也受监管，包括尚未启动 AI CLI 时的连接失败。

## 设置与提醒

所有全局设置按 **Apply** 生效，**Cancel** 恢复上次保存值并返回托盘。会话操作和生成接入命令即时执行，不会保存表单草稿。

| 设置 | 默认 | 可配置范围 |
| --- | --- | --- |
| Scan interval | 10 秒 | 1–3600 秒 |
| Quiet threshold | 20 秒 | 1–86400 秒 |
| Tab header flash | 6 秒 | 0–300 秒，0 表示静态标记 |
| Also pulse the tray icon briefly | 开启 | 可关闭；固定 1 秒、至少间隔 60 秒 |

扫描间隔决定多久评估一次安静状态；安静阈值决定多久没有有效输出才需要关注，因此安静提示最多会再等待一个扫描间隔。hook 状态通过事件通道及时处理，不等待扫描。桥接心跳和标题动画使用独立时钟，不受扫描间隔限制。

| 标记 | 含义 |
| --- | --- |
| `✓` | 顶层本轮回复结束，等待下一步；不保证整个任务成功 |
| `?` | 明确等待输入、授权或确认 |
| `~` | 暂无有效输出，可能在计算或等待网络；不能据此断言完成或断线 |
| `!` | 明确的终止错误或 SSH 异常退出，列表显示原因 |

状态符与 `·` 每 500 毫秒交替，达到配置时长后保留静态标记。同一轮最多主动提醒一次；状态升级只更新标记，不重复闪。下一次顶层请求开始后清除旧状态并重新布防。子代理结束和普通 idle 通知不独立触发提醒。

- **Mark read**：清除当前标记，本轮不重新闪；后续状态升级可以显示新的静态标记。
- **Ignore turn**：清除并忽略本轮全部后续标记，下轮请求自动恢复。
- **Pause tab / Resume**：暂停跨越后续轮次，直到手动恢复；恢复时不补闪旧事件。

托盘菜单显示待关注计数和至多 30 个会话，全部会话可在 Monitoring 页分批查看。多会话同时结束只合并为一次托盘短暂变色，冷却期内不排队补闪。

Windows Terminal 的手动固定标题或 `suppressApplicationTitle` 可以遮住应用设置的标题；需要解除覆盖才能显示字符标记，托盘列表仍可查看状态。监控保留应用最近一次标题，不向终端输入命令、不抢焦点、不主动切换 tab。[Windows Terminal 标题规则](https://learn.microsoft.com/en-us/windows/terminal/tutorials/tab-title)

## 故障与数据

本地保存路径是 `%APPDATA%\com.traylnks.launcher\monitor\`。仅保存当前会话的身份、标题、主机、状态、输出活动计数及提醒策略；不保存终端正文、用户输入或完整 hook payload，不参与启动器目录的云同步。

远端状态通过当前 SSH 终端流中的专用控制帧回传，由本地桥接解析并移除；不额外开放端口。脚本退出或存储不可用不会让 hook 阻塞 CLI；心跳失效显示 **Observer unavailable**，不伪装成任务结束或确定断链。TrayLnks 重启后恢复策略及静态状态，不补闪历史事件。关闭受监管 shell 后清理该会话的运行记录。

## 验证

```bash
python3 -m unittest discover -s tools/terminal-agent -p 'test_*.py' -v
cargo fmt --all --manifest-path src-tauri/Cargo.toml -- --check
cargo xwin clippy --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc -- -D warnings
cargo xwin test --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --lib --no-run
# 在 Windows / WSL interop 中运行输出的 traylnks_lib-*.exe
npm --prefix tools/ui-tests ci
npm --prefix tools/ui-tests exec -- playwright install chromium
npm --prefix tools/ui-tests test
```

UI 测试通过真实 Chromium 和隔离的 Tauri IPC fixture 检查草稿保存、撤销、错误恢复、会话操作、键盘和窄窗口。若使用已有浏览器，可设置 `CHROMIUM_PATH`。PTY 集成测试使用临时进程和配置，不安装到用户的工具配置中。还可运行真实 OpenSSH 客户端与隔离的本地 SSH 服务集成测试，覆盖远端 hook 回传和连接中断：

```bash
uv run --with asyncssh python -m unittest discover -s tools/terminal-agent -p test_ssh.py -v
```

该测试不读取用户 SSH 配置或密钥。目标主机的实际认证与网络策略仍需在部署环境验收。
