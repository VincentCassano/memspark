# Before you run MemSpark · 内存优化 cleanup

[简体中文](safety.md) | **English** · [Home](../README.en.md)

## Save your work

Real cleanup requires administrator privileges. To reach the 25% memory usage target, MemSpark may close or forcibly terminate candidate applications after working-set trimming and system release. Unsaved documents, browser content, and running tasks can be affected.

Save your work and finish tasks you do not want interrupted. Do not run cleanup during games, meetings, recordings, builds, rendering, or other important work.

## Process exclusions do not protect all application data

The code excludes MemSpark itself and internally listed critical system processes such as `System`, `csrss.exe`, and `lsass.exe`. The targeted termination stage also excludes some system services and virtualization-related processes.

These rules do not protect every application. In particular, do not assume a foreground window will stay open: the current termination stage has no general foreground exclusion check. An internal list also cannot guarantee that task dependencies remain unaffected.

## Pauses and rising memory usage

After working sets and caches are released, applications may need to read their data again. Switching windows, restoring browser tabs, or loading large files can cause brief pauses. Usage can rise again as you work; repeated cleanup just to keep the number low is unnecessary.

## Preview and configuration limits

`trim --dry-run --verbose` does not release memory or terminate processes. It shows working-set candidates and skip reasons, not a complete list of processes that might be terminated later. CLI previews still write reports and logs.

The current `trim` system release flow is not controlled by the `advanced_cleanup` configuration switches. Disabling them does not guarantee that a run will skip system release or process termination.

## Files and logs

Cleanup does not deliberately delete application data files, inject code into processes, or rewrite another process's memory. Terminating an application can still lose unsaved work.

Reports and developer logs are stored under the user profile and can include process names and PIDs. Review them before sharing. The report-history and log cleanup buttons in Settings delete the corresponding files; keep any records you need before using them.
