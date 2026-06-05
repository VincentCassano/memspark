# MemSpark

MemSpark 是一款面向 Windows 的轻量级内存优化工具。当前版本只保留一个模式：`内存优化`。唯一目标是尽量把内存占用压到 25% 或以下。

MemSpark 不注入进程，不修改其他进程内存，也不删除程序数据。为达成 25% 目标，它会依次执行工作集整理、系统级内存释放，并在仍未达标时处理非系统后台候选进程。该过程可能导致未保存数据丢失或短暂卡顿，请在理解代价后使用。

## 下载

当前发布版本：`v0.1.4`

下载地址：[GitHub Releases](https://github.com/VincentCassano/memspark/releases/latest)

Release 页面提供 3 个 Windows x64 文件：

- `MemSpark-GUI-v0.1.4-windows-x64.exe`：图形界面版本，适合直接双击使用。
- `MemSpark-CLI-v0.1.4-windows-x64.exe`：命令行版本，适合在 PowerShell、Windows Terminal 或脚本中使用。
- `MemSpark-v0.1.4-windows-x64.zip`：完整包，包含 `memspark-ui.exe` 和 `memspark-cli.exe`。

## 功能定位

- 只保留 `trim` 一个优化入口。
- 非 dry-run 执行会自动请求管理员权限。
- CLI 和 Slint GUI 共用 `memspark-core`。
- GUI 和 CLI 都已嵌入 MemSpark 程序图标。
- GUI 支持深色和浅色主题。
- 默认保存最近一次优化报告到 `%APPDATA%\MemSpark\last_report.json`，覆盖前会把上一份报告归档到 `%APPDATA%\MemSpark\history`。
- 每次优化会写入开发者日志到 `%APPDATA%\MemSpark\logs`，日志包含详细文本报告、统计信息、进程结果和原始 JSON。
- 系统关键进程和 MemSpark 自身属于内部硬边界，不提供用户可编辑进程名单。

## GUI.exe 使用方式

### 直接运行单文件 GUI

1. 从 [Releases](https://github.com/VincentCassano/memspark/releases/latest) 下载 `MemSpark-GUI-v0.1.4-windows-x64.exe`。
2. 双击运行。
3. 在 Dashboard 页面点击 `内存优化`。
4. Windows 弹出 UAC 管理员授权时，选择允许。
5. 优化结束后，在 Report 页面查看本次结果。

GUI 单文件版本不需要同目录额外放置 CLI 程序。管理员释放会由 GUI 自身启动提权子进程完成。

### 使用完整 zip 包中的 GUI

1. 下载 `MemSpark-v0.1.4-windows-x64.zip`。
2. 解压到任意目录。
3. 双击运行解压目录中的 `memspark-ui.exe`。
4. 点击 Dashboard 页面右上角的 `内存优化`。

GUI 页面说明：

- Dashboard：查看当前内存占用、可用内存、系统缓存、进程数量，并执行内存优化。
- Report：查看最近一次优化报告、最终内存占用、可用内存变化、处理进程和跳过原因。
- Settings：切换语言和主题，打开报告目录，清理历史报告和开发者日志。
- About：查看版本、许可和安全边界说明。

## CLI.exe 使用方式

### 直接运行单文件 CLI

从 [Releases](https://github.com/VincentCassano/memspark/releases/latest) 下载 `MemSpark-CLI-v0.1.4-windows-x64.exe` 后，在该文件所在目录打开 PowerShell：

```powershell
.\MemSpark-CLI-v0.1.4-windows-x64.exe status
.\MemSpark-CLI-v0.1.4-windows-x64.exe trim --dry-run
.\MemSpark-CLI-v0.1.4-windows-x64.exe trim --dry-run --verbose
.\MemSpark-CLI-v0.1.4-windows-x64.exe trim
.\MemSpark-CLI-v0.1.4-windows-x64.exe report last
```

常用命令：

- `status`：查看当前内存状态。
- `trim --dry-run`：只预览将会处理的候选进程，不实际释放内存。
- `trim --dry-run --verbose`：预览并显示更完整的跳过原因。
- `trim`：执行内存优化。非管理员运行时会自动请求管理员权限。
- `report last`：读取最近一次优化报告。
- `config init`：初始化默认配置文件。
- `config show`：显示当前配置。

CLI 支持通过 `--config <path>` 指定配置文件，例如：

```powershell
.\MemSpark-CLI-v0.1.4-windows-x64.exe --config .\config.toml trim
```

### 使用完整 zip 包中的 CLI

下载并解压 `MemSpark-v0.1.4-windows-x64.zip` 后，在解压目录运行：

```powershell
.\memspark-cli.exe status
.\memspark-cli.exe trim --dry-run
.\memspark-cli.exe trim
.\memspark-cli.exe report last
```

## 开发版 CLI 用法

```bash
cargo run -p memspark-cli -- status
cargo run -p memspark-cli -- trim --dry-run
cargo run -p memspark-cli -- trim --dry-run --verbose
cargo run -p memspark-cli -- trim
cargo run -p memspark-cli -- config init
cargo run -p memspark-cli -- config show
cargo run -p memspark-cli -- report last
```

本地 release 构建后：

```powershell
.\target\release\memspark.exe status
.\target\release\memspark.exe trim --dry-run
.\target\release\memspark.exe trim --dry-run --verbose
.\target\release\memspark.exe trim
.\target\release\memspark.exe report last
```

## 开发版 GUI 用法

```bash
cargo run -p memspark-ui
```

开发版 GUI 当前包含 Dashboard、Report、Settings、About 页面。Dashboard 直接提供“内存优化”执行入口，优化任务在线程中执行，界面会在执行期间继续刷新内存占用率。

## 配置与报告

默认配置路径：

```text
%APPDATA%\MemSpark\config.toml
```

最近一次报告路径：

```text
%APPDATA%\MemSpark\last_report.json
```

历史报告目录：

```text
%APPDATA%\MemSpark\history
```

开发者日志目录：

```text
%APPDATA%\MemSpark\logs
```

Report 页面会围绕 25% 目标展示最终内存占用、可用内存变化、工作集处理数量和系统释放状态。
Settings 页面可以清理历史报告目录中的 JSON 报告，也可以清理开发者日志目录中的 `.log` 文件。

可以通过 `--config <path>` 指定 CLI 配置文件。

## 优化流程

1. 读取优化前内存快照。
2. 枚举进程并整理符合条件的后台工作集。
3. 使用管理员权限执行系统级内存释放。
4. 若仍高于 25%，继续处理非系统后台候选进程。
5. 将上一份最近报告复制到历史目录。
6. 写入新的最近优化报告。
7. 写入开发者日志。

## 编译

```bash
cargo build --release
```

## 开发验证

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo test
cargo build --release
```

## 文档

- [用户手册](docs/user-manual.md)
- [原理说明](docs/principle.md)
- [安全边界](docs/safety.md)

## 免责声明

MemSpark 会尽可能向 25% 内存占用目标推进，但 Windows 内核、驱动、硬件保留和不可终止进程仍可能限制最终结果。请在理解工作集整理、系统缓存释放和候选进程处理代价后使用。

## License

MIT
