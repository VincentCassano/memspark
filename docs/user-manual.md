# MemSpark 用户手册

## CLI 用法

查看状态：

```bash
memspark status
```

预览内存优化：

```bash
memspark trim --dry-run
```

查看完整跳过明细：

```bash
memspark trim --dry-run --verbose
```

执行内存优化：

```bash
memspark trim
```

初始化配置：

```bash
memspark config init
```

查看最近报告：

```bash
memspark report last
```

release 构建后的 PowerShell 用法：

```powershell
.\target\release\memspark.exe status
.\target\release\memspark.exe trim --dry-run
.\target\release\memspark.exe trim --dry-run --verbose
.\target\release\memspark.exe trim
.\target\release\memspark.exe report last
```

## GUI 用法

```bash
cargo run -p memspark-ui
```

Dashboard 页面展示当前物理内存、可用内存、Commit、System Cache 和进程数量，并直接提供“内存优化”执行入口。Report 页面展示最近一次报告。Settings 页面管理报告路径和界面选项。About 页面展示安全边界。

## 配置文件

默认路径：

```text
%APPDATA%\MemSpark\config.toml
```

CLI 可通过全局参数指定：

```bash
memspark --config D:\path\config.toml config show
```

## 报告查看

最近一次报告默认保存为 JSON：

```text
%APPDATA%\MemSpark\last_report.json
```

再次优化时，上一份最近报告会被复制到历史目录：

```text
%APPDATA%\MemSpark\history
```

每次优化还会写入开发者日志：

```text
%APPDATA%\MemSpark\logs
```

开发者日志包含详细文本报告、核心统计、跳过/失败原因统计、每个进程的处理结果和原始 JSON。

CLI 查看：

```bash
memspark report last
```

GUI 在 Report 页面查看。该页面会围绕 25% 目标展示最终内存占用、可用内存变化、工作集处理数量和系统释放状态。Settings 页面提供“清理历史报告”和“清理开发日志”按钮，分别用于删除历史目录中的 JSON 报告和日志目录中的 `.log` 文件。

## 常见问题

### MemSpark 会删除数据吗？

不会删除程序数据，不注入进程，也不修改其他进程内存。为达成 25% 目标，它可能关闭或终止非系统后台候选进程，因此未保存数据可能丢失。

### 为什么释放后程序重新打开会卡？

被修剪页面重新访问时可能发生缺页，需要从文件或页面文件重新调入。

### 为什么不默认自动定时清理？

定时清理容易造成周期性缺页和卡顿。MemSpark 默认只在用户主动执行时运行。

### 还有其他优化模式吗？

没有。当前版本只保留“内存优化”，唯一目标是内存占用不高于 25%。
