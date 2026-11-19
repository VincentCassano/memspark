# 轻存 · MemSpark 使用手册

**简体中文** | [English](user-manual.en.md) · [返回首页](../README.md)

## 第一次运行

从 [Releases](https://github.com/VincentCassano/memspark/releases/latest) 下载 Windows x64 GUI 单文件版，直接运行即可。GUI 会自行启动提权子进程，不需要在旁边放置 CLI。

执行清理前先保存工作。程序为了接近 25% 的内存占用目标，可能关闭或强制结束其他程序。正在使用的前台程序也不能视为一定受保护，具体行为见[使用前须知](safety.md)。

## 图形界面

| 页面 | 用途 |
| --- | --- |
| Dashboard | 查看物理内存、可用内存、Commit、系统缓存和进程数量；点击「内存优化」执行清理 |
| Report | 查看最近一次报告，包括内存变化、进程结果和跳过原因 |
| Settings | 切换语言和主题，管理报告保存位置，清理历史报告及开发日志 |
| About | 查看版本、许可和使用说明 |

清理需要管理员权限。出现 UAC 窗口时，同意授权才会继续；清理结束后到 Report 查看结果。

## 命令行

以下命令在完整包解压目录内运行。单文件 CLI 的文件名不同，请替换 `memspark-cli.exe`。

| 命令 | 用途 |
| --- | --- |
| `.\memspark-cli.exe status` | 查看内存状态和进程数量 |
| `.\memspark-cli.exe trim --dry-run` | 预览工作集候选，不执行清理 |
| `.\memspark-cli.exe trim --dry-run --verbose` | 预览时显示完整跳过明细 |
| `.\memspark-cli.exe trim` | 执行清理；需要时自动请求管理员权限 |
| `.\memspark-cli.exe report last` | 查看最近一次报告 |
| `.\memspark-cli.exe config init` | 创建默认配置；已有文件会保留 |
| `.\memspark-cli.exe config show` | 显示当前生效的配置 |

`config init --force` 会用默认配置覆盖已有文件，使用前请自行备份。

`--dry-run` 适合先了解工作集候选和跳过原因。它不会执行内存释放，也不会结束进程，但会保存报告和日志，并替换「最近一次报告」。实际运行中的进程状态会变化，预览结果也不包含后续结束进程阶段的完整候选名单。

## 配置文件

默认配置路径为 `%APPDATA%\MemSpark\config.toml`。文件不存在时使用内置默认值。

通过 `--config` 指定其他路径：

```powershell
.\memspark-cli.exe --config .\config.toml config init
.\memspark-cli.exe --config .\config.toml config show
.\memspark-cli.exe --config .\config.toml trim --dry-run
```

配置中的 `advanced_cleanup` 不是当前清理入口的系统释放开关。实际 `trim` 使用固定的目标清理流程，不能通过将这些字段设为 `false` 来保证只整理工作集。

## 报告与日志

默认位置：

```text
%APPDATA%\MemSpark\
├── config.toml
├── last_report.json
├── history\
└── logs\
```

保存新报告前，程序会将上一份报告复制到 `history`。`logs` 中的开发日志包含详细文本报告、统计、每个进程的处理结果和原始 JSON。

Settings 中的清理按钮分别删除历史目录内的 JSON 报告和日志目录内的 `.log` 文件。提交问题时，建议提供版本、Windows 版本、执行方式，以及删去私人信息后的报告或日志。

## 结果怎么看

优先看清理后的物理内存占用、可用内存变化和处理结果。单个进程工作集的减少量不能简单当作整机实际释放量，两者统计口径不同。

没有达到 25% 不一定代表执行失败：系统、驱动和无法处理的进程仍会占用内存。清理后占用回升也很常见，因为程序会继续使用数据。原理见[清理原理](principle.md)。
