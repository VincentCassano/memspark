# 轻存 · MemSpark

**简体中文** | [English](README.en.md)

Windows 内存清理工具，提供图形界面和命令行。可以查看内存状态、手动执行清理，并查看每次清理前后的变化。

> 使用前请保存工作。清理会先整理进程工作集和系统缓存；如果内存占用仍高于 25%，还可能关闭或强制结束程序，包括浏览器。25% 是程序的清理目标，不是每台电脑都能达到的结果。

[下载](https://github.com/VincentCassano/memspark/releases/latest) · [使用手册](docs/user-manual.md) · [清理原理](docs/principle.md) · [使用前须知](docs/safety.md)

## 下载与使用

在 [Releases](https://github.com/VincentCassano/memspark/releases/latest) 选择 Windows x64 文件：

| 文件 | 用途 |
| --- | --- |
| `MemSpark-GUI-*-windows-x64.exe` | 图形界面，下载后直接运行 |
| `MemSpark-CLI-*-windows-x64.exe` | 命令行，适合 PowerShell 或脚本调用 |
| `MemSpark-*-windows-x64.zip` | 同时包含图形界面和命令行程序 |

初次使用建议选择 GUI 单文件版，不需要另外下载 CLI。

1. 保存文档，结束不希望被打断的任务。
2. 运行 GUI，在 Dashboard 页面查看内存状态。
3. 点击「内存优化」。实际清理需要管理员权限，Windows 会弹出 UAC 授权窗口。
4. 完成后到 Report 页面查看结果。Settings 页面可以切换中英文、深浅主题，以及管理报告和日志。

## 清理会做什么

清理只有一个模式，按以下顺序执行：

1. 整理符合条件的进程工作集，让 Windows 回收可移出的物理内存页面。
2. 执行系统级释放，包括系统工作集、修改页列表和待机列表的处理。
3. 如果仍未达到 25% 目标，尝试关闭候选程序；必要时强制终止。
4. 记录内存变化、处理结果和失败或跳过原因。

工作集缩小后，程序再次访问数据时可能需要把页面调回内存，因此占用可能回升，也可能出现短暂卡顿。这不表示程序本身需要的内存变少了。详细说明见[清理原理](docs/principle.md)。

## 命令行

以下示例使用完整包中的 `memspark-cli.exe`。单文件 CLI 请换成下载的实际文件名。

```powershell
# 查看内存状态
.\memspark-cli.exe status

# 预览工作集候选和跳过原因，不执行清理
.\memspark-cli.exe trim --dry-run --verbose

# 执行清理，可能关闭或强制结束程序
.\memspark-cli.exe trim

# 查看最近一次报告
.\memspark-cli.exe report last
```

`--dry-run` 不执行内存释放或结束进程，但仍会保存报告和日志。它不是实际清理过程中所有可能被结束程序的完整名单。

配置文件用法和更多命令见[使用手册](docs/user-manual.md)。

## 报告存在哪里

默认保存在 `%APPDATA%\MemSpark\`：

| 路径 | 内容 |
| --- | --- |
| `config.toml` | 配置文件，可用 `config init` 创建 |
| `last_report.json` | 最近一次报告 |
| `history\` | 之前的报告 |
| `logs\` | 详细日志，包含进程结果和原始 JSON |

报告和日志可能包含进程名称、PID 等信息。提交问题前，请先检查并删去不想公开的内容。

## 从源码运行

需要 Windows、Rust 稳定工具链和 MSVC 构建工具。项目使用 Rust、Slint 和 Windows API。

```powershell
git clone https://github.com/VincentCassano/memspark.git
cd memspark
cargo run -p memspark-ui
```

运行命令行预览：

```powershell
cargo run -p memspark-cli -- trim --dry-run --verbose
```

构建发布版本：

```powershell
cargo build --release
```

生成的程序是 `target\release\memspark-ui.exe` 和 `target\release\memspark.exe`。发布包中的 CLI 会重命名为 `memspark-cli.exe`。

## 项目结构

- `crates/memspark-core`：内存状态、进程处理、清理策略和报告。
- `crates/memspark-cli`：命令行入口。
- `crates/memspark-ui`：Slint 图形界面。
- `docs`：使用手册、原理和使用注意事项。

发现问题可以提交 [Issue](https://github.com/VincentCassano/memspark/issues)。开发约定见 [CONTRIBUTING.md](CONTRIBUTING.md)，安全问题见 [SECURITY.md](SECURITY.md)（这两份文档目前为英文）。

## 许可

[MIT](LICENSE)。
