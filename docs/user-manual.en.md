# MemSpark · 轻存 user guide

[简体中文](user-manual.md) | **English** · [Home](../README.en.md)

## First run

Download the Windows x64 standalone GUI from [Releases](https://github.com/VincentCassano/memspark/releases/latest) and run it. The GUI starts its own elevated child process; no separate CLI is needed.

Save your work before cleanup. To reach its 25% memory usage target, MemSpark may close or forcibly terminate other applications. Do not assume an application is protected because it is in the foreground. See [Before you run cleanup](safety.en.md).

## GUI

| Page | Use |
| --- | --- |
| Dashboard | View physical memory, available memory, commit, system cache, and process count; start memory optimization |
| Report | Review the last report, including memory changes, process results, and skip reasons |
| Settings | Change language and theme, manage report locations, and clear report history and developer logs |
| About | View the version, license, and usage notes |

Cleanup requires administrator privileges. Accept the Windows UAC prompt to continue, then check Report when the run finishes.

## Command line

Run these commands from the extracted package directory. Substitute the actual downloaded filename if you use the standalone CLI.

| Command | Use |
| --- | --- |
| `.\memspark-cli.exe status` | Show memory status and process count |
| `.\memspark-cli.exe trim --dry-run` | Preview working-set candidates without cleanup |
| `.\memspark-cli.exe trim --dry-run --verbose` | Include all skipped processes in the preview |
| `.\memspark-cli.exe trim` | Run cleanup; request administrator privileges if needed |
| `.\memspark-cli.exe report last` | Read the last report |
| `.\memspark-cli.exe config init` | Create default configuration, preserving an existing file |
| `.\memspark-cli.exe config show` | Show the effective configuration |

`config init --force` overwrites an existing file with defaults. Back it up first if you need to keep it.

Use `--dry-run` to inspect working-set candidates and skip reasons. It does not release memory or terminate processes, but it saves reports and logs and replaces the last report. Process state can change before a real run, and the preview does not list all candidates for the later termination stage.

## Configuration

The default path is `%APPDATA%\MemSpark\config.toml`. Built-in defaults are used when the file does not exist.

Choose another path with `--config`:

```powershell
.\memspark-cli.exe --config .\config.toml config init
.\memspark-cli.exe --config .\config.toml config show
.\memspark-cli.exe --config .\config.toml trim --dry-run
```

The `advanced_cleanup` fields do not control system release in the current cleanup entry point. `trim` uses a fixed target-based release flow. Setting these fields to `false` does not restrict cleanup to working-set trimming.

## Reports and logs

Default locations:

```text
%APPDATA%\MemSpark\
├── config.toml
├── last_report.json
├── history\
└── logs\
```

Before saving a new report, MemSpark copies the previous one to `history`. Developer logs in `logs` contain a detailed text report, statistics, individual process results, and raw JSON.

The cleanup buttons in Settings delete JSON files from the history directory and `.log` files from the log directory. When reporting a problem, include your MemSpark version, Windows version, how you ran it, and a report or log with private information removed.

## Reading the result

Start with physical memory usage after cleanup, the change in available memory, and process results. The sum of individual working-set reductions is not the same measurement as memory released across the whole system.

Missing the 25% target does not necessarily mean the operations failed. The system, drivers, and processes that cannot be handled still use memory. Usage can rise again as applications access their data. See [How cleanup works](principle.en.md).
