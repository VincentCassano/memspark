# How MemSpark · 内存优化 cleanup works

[简体中文](principle.md) | **English** · [Home](../README.en.md)

## Working sets

A process working set contains the pages currently resident in physical memory. Allocated virtual memory does not have to be resident all at once. Windows manages those pages according to access patterns and memory pressure.

MemSpark requests working-set trimming through Windows APIs. It tries `EmptyWorkingSet` first, then `SetProcessWorkingSetSizeEx` if the first call fails. Windows continues managing removed pages; trimming does not remove the application's data structures or its need for virtual memory.

## Why usage drops and rises again

A smaller working set reduces the physical memory currently resident for that process. When the application accesses the pages again, Windows brings them back into its working set. A page may still be in RAM or may need to be read from a file or the page file. Disk reads can cause a brief pause.

A drop in usage is a change in memory state. It does not by itself show that an application runs faster, and it does not fix a memory leak in the application.

## System release

After trimming process working sets, MemSpark schedules system release operations based on current memory usage:

- Trim system working sets.
- Flush the modified page list.
- Purge low-priority standby pages and the standby list.

These operations require the relevant Windows privileges. Their order and number of passes vary with memory pressure. Individual operations can fail because of permissions or system restrictions; reports and logs record the results.

## The 25% target and process termination

The current implementation targets physical memory usage of 25% or less. If system release does not reach it, MemSpark moves to candidate process closure: it requests window closure, forcibly terminates processes when needed, and attempts further system release.

Ending a process discards its running state. That is different from trimming its working set, and unsaved work can be lost. The full cleanup flow does more than adjust caches. Internal exclusion rules apply, but the closure stage does not provide a general foreground-window safeguard.

The 25% result is not guaranteed. System reservations, drivers, the kernel, and processes that cannot be handled all affect final usage.

## Report measurements

The reduction in process working sets and the change in system-wide available memory are different measurements. Other applications keep running during cleanup, so the before-and-after snapshots describe the system at those moments rather than an isolated performance benchmark.

Judge cleanup by application responsiveness and the work you need to do next. Repeating it just to lower the usage number may not help. For commands and GUI steps, see the [user guide](user-manual.en.md).
