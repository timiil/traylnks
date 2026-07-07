# PRD — Windows Tray Link Launcher

> 版本：v0.1  
> 目标平台：Windows 10 / 11  
> 建议技术栈：Tauri v2

## 1. 产品目标

开发一款轻量级 Windows 托盘启动器。

主程序启动后常驻系统 Tray，通过扫描本地监听目录，自动生成多级菜单：

- 文件夹 → Sub Menu
- `.lnk` → Menu Item
- 同名图标文件 → 菜单图标

核心原则：

> 文件系统即配置，本地目录是唯一运行源。

---

## 2. 本地优先

Tray Menu 只从本地监听目录生成。

网盘路径只作为不同 Windows PC 之间的同步中介，不直接作为菜单数据源。

即使：

- 无网络
- 网盘客户端未启动
- 网盘路径不可访问
- 未配置网盘

本地菜单仍必须正常：

- 显示
- 刷新
- 执行 `.lnk`

---

## 3. 配置项

至少支持：

- 本地监听路径
- 网盘授权路径（可选）

示例：

```yaml
watch_path: "D:\\TrayLauncher"
cloud_path: "D:\\OneDrive\\TrayLauncherSync"
```

v0.1 允许先支持单个本地监听路径。

---

## 4. 菜单映射规则

例如：

```text
TrayLauncher/
├── Development/
│   ├── Terminal.lnk
│   └── Codex.lnk
└── Projects/
    └── Snooker/
        └── Open Workspace.lnk
```

生成：

```text
Development >
    Terminal
    Codex

Projects >
    Snooker >
        Open Workspace
```

规则：

- 文件夹递归映射为 Sub Menu
- `.lnk` 去掉扩展名后作为菜单名称
- 点击菜单项时，由 Windows Shell 原生执行 `.lnk`
- 不自行解析并重建 `.lnk` 的命令

---

## 5. 图标规则

支持：

- `.ico`
- `.png`
- `.jpg`
- `.jpeg`

假设：

```text
Codex.lnk
Codex.ico
Codex.png
Codex.jpg
```

图标优先级：

```text
.ico
>
.png
>
.jpg
>
.jpeg
>
.lnk 自身图标
>
目标程序图标
>
系统默认图标
```

文件夹图标规则相同。

例如：

```text
Development/
Development.png
```

`Development.png` 作为该 Sub Menu 图标。

图标文件本身不生成菜单项。

---

## 6. `.station` 工作站过滤

目录中允许存在：

```text
.station
```

它是 UTF-8 纯文本文件，每行填写一个允许显示该目录的 Windows 主机名称。

例如：

```text
DESKTOP-PAMJPBD
TIM-LAPTOP
```

规则：

- 不区分大小写
- 精确匹配主机名称
- 忽略空行
- 允许以 `#` 开头的注释行
- `.station` 本身不显示在菜单中

如果当前主机名匹配：

- 目录正常显示
- 目录正常同步

如果不匹配：

- 目录仍正常同步
- 本机 Tray Menu 不显示该目录及其整个子树

如果目录没有 `.station`：

- 所有工作站正常显示

如果 `.station` 为空：

- 所有工作站均不显示该目录

父目录 `.station` 不匹配时，整个子树直接停止菜单扫描。

---

## 7. 普通文件规则

监听目录允许自由存放：

- `.md`
- `.txt`
- `.json`
- `.yaml`
- `.ps1`
- `.bat`
- 以及其他普通文件

主程序采用白名单渲染：

```text
文件夹 → 菜单
.lnk   → 菜单项
```

其他普通文件：

- 不渲染
- 不生成菜单项
- 不报错
- 不影响目录结构

例如：

```text
GPU Tools/
├── .station
├── README.md
├── notes.txt
├── Start SAM3.lnk
└── Start llama.cpp.lnk
```

Tray Menu 只显示：

```text
GPU Tools >
    Start SAM3
    Start llama.cpp
```

普通文件仍可跟随目录进行网盘同步。

---

## 8. 网盘同步原则

网盘只负责传输和共享同一套目录内容。

逻辑模型：

```text
PC-A Local
    ↕
Cloud Sync Folder
    ↕
PC-B Local
```

其中：

- Local 是唯一运行源
- Cloud 是同步中介
- `.station` 决定某目录在哪些 Windows 主机上显示

本项目 v0.1 不自行实现：

- 网盘 OAuth
- 云端账号系统
- OneDrive / Dropbox 协议
- SaaS 同步服务

优先依赖现有网盘客户端提供本地同步目录。

---

## 9. 文件监听

主程序启动后：

1. 扫描本地目录
2. 构建 Menu Tree
3. 创建 Tray Menu
4. 启动 File Watcher

监听：

- 创建
- 删除
- 修改
- 重命名

包括：

- `.lnk`
- 文件夹
- 图标文件
- `.station`

建议使用约 `500ms` debounce，避免连续文件事件导致频繁重建菜单。

---

## 10. Tray 行为

主程序：

- 启动后常驻 Tray
- 默认不显示主窗口
- 支持打开动态菜单
- 支持手动刷新
- 支持打开监听目录
- 支持 Settings
- 支持 Exit

建议 Tray Menu 底部：

```text
────────────
Refresh
Open Launcher Folder
Settings
Exit
```

---

## 11. Settings

v0.1 最少提供：

### General

- 开机启动
- 启动后最小化

### Paths

- Local Watch Path
- Cloud Authorized Path（可选）

### Station

显示当前 Windows 主机名称，并提供复制能力。

### Diagnostics

显示：

- App Version
- Config Path
- Watch Path
- Cloud Path
- Watcher Status
- Last Scan Time

---

## 12. 容错要求

任何单个错误都不得导致整个程序停止：

- 损坏的 `.lnk`
- 损坏的图标
- 无法读取的目录
- 网盘离线
- `.station` 格式异常

应尽量降级处理，并保持其他菜单正常可用。

---

## 13. v0.1 MVP

必须实现：

- Windows 10 / 11
- Tauri v2
- Tray 常驻
- 本地监听路径
- 多级文件夹菜单
- `.lnk` 启动项
- 同名图标解析
- `.station` 主机过滤
- 普通文件不渲染
- 手动刷新
- Settings
- 单实例
- 基本日志

后续版本再增加：

- 自动网盘同步调度
- 多监听路径
- 排序前缀隐藏
- `.nosync`
- 分隔线
- 更完整的冲突处理

---

## 14. 核心产品原则

```text
本地目录是唯一运行源
文件夹就是菜单
文件夹层级就是菜单层级
.lnk 就是启动项
同名图片就是图标
.station 决定菜单在哪些工作站显示
普通文件可以存在，但不渲染
网盘只是同步传输层
Windows Shell 就是执行器
```
