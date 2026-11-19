# MemSpark · 轻存

[简体中文](README.md) | **English**

A Windows memory cleanup tool with a GUI and a command-line interface. Check memory usage, run cleanup manually, and review what changed after each run.

> Save your work first. Cleanup starts with process working sets and system caches. If memory usage stays above 25%, it may also close or forcibly terminate applications, including browsers. The 25% threshold is a cleanup target, not a guaranteed result.

[Download](https://github.com/VincentCassano/memspark/releases/latest) · [User guide](docs/user-manual.en.md) · [How cleanup works](docs/principle.en.md) · [Before you run cleanup](docs/safety.en.md)

## Download and run

Choose a Windows x64 file from [Releases](https://github.com/VincentCassano/memspark/releases/latest):

| File | Use |
| --- | --- |
| `MemSpark-GUI-*-windows-x64.exe` | Standalone GUI; download and run |
| `MemSpark-CLI-*-windows-x64.exe` | CLI for PowerShell or scripts |
| `MemSpark-*-windows-x64.zip` | Package containing both programs |

For a first run, use the standalone GUI. It does not need a separate CLI download.

1. Save open documents and finish tasks you do not want interrupted.
2. Open the GUI and check memory usage on Dashboard.
3. Click the memory optimization button. Cleanup requires administrator privileges and opens a Windows UAC prompt.
4. Review the result on Report. Settings lets you switch between Chinese and English, choose a light or dark theme, and manage reports and logs.

## What cleanup does

There is one cleanup mode. It runs these steps in order:

1. Trim eligible process working sets so Windows can reclaim removable physical memory pages.
2. Run system release operations on working sets, the modified page list, and standby lists.
3. If usage still exceeds the 25% target, try closing candidate applications and forcibly terminate them if needed.
4. Record memory changes, process results, and failures or skipped operations.

When an application accesses trimmed pages again, Windows may need to bring them back into memory. Usage can rise again, and the application may pause briefly. A smaller working set does not mean the application needs less memory. See [How cleanup works](docs/principle.en.md).

## Command line

These examples use `memspark-cli.exe` from the ZIP package. For the standalone CLI, substitute the downloaded filename.

```powershell
# Check memory usage
.\memspark-cli.exe status

# Preview working-set candidates and skip reasons without running cleanup
.\memspark-cli.exe trim --dry-run --verbose

# Run cleanup; applications may be closed or forcibly terminated
.\memspark-cli.exe trim

# Read the last report
.\memspark-cli.exe report last
```

`--dry-run` does not release memory or terminate processes, but it still saves reports and logs. Its output is not a complete list of applications that might be terminated during a real run.

See the [user guide](docs/user-manual.en.md) for configuration and more commands.

## Reports and configuration

The default directory is `%APPDATA%\MemSpark\`:

| Path | Contents |
| --- | --- |
| `config.toml` | Configuration; create it with `config init` |
| `last_report.json` | Most recent report |
| `history\` | Earlier reports |
| `logs\` | Detailed logs, including process results and raw JSON |

Reports and logs can contain process names and PIDs. Review them and remove anything you do not want to publish before attaching them to an issue.

## Run from source

Use Windows with the stable Rust toolchain and MSVC build tools. The project uses Rust, Slint, and Windows APIs.

```powershell
git clone https://github.com/VincentCassano/memspark.git
cd memspark
cargo run -p memspark-ui
```

Preview cleanup through the CLI:

```powershell
cargo run -p memspark-cli -- trim --dry-run --verbose
```

Build release binaries:

```powershell
cargo build --release
```

The binaries are `target\release\memspark-ui.exe` and `target\release\memspark.exe`. The packaged CLI is renamed to `memspark-cli.exe`.

## Project layout

- `crates/memspark-core`: memory status, process handling, cleanup policy, and reports.
- `crates/memspark-cli`: command-line entry point.
- `crates/memspark-ui`: Slint GUI.
- `docs`: usage, implementation notes, and cleanup precautions.

Report bugs through [Issues](https://github.com/VincentCassano/memspark/issues). See [CONTRIBUTING.md](CONTRIBUTING.md) for development conventions and [SECURITY.md](SECURITY.md) for security reports.

## License

[MIT](LICENSE).
