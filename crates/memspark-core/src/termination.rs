use crate::policy::{
    browser_processes, same_process_name, system_boundary_processes, vm_stack_processes,
};
use crate::{enumerate_processes, winapi, Result};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessReleaseReport {
    pub dry_run: bool,
    pub enabled: bool,
    pub allow_force_kill: bool,
    pub duration_ms: u64,
    pub scanned_count: usize,
    pub matched_count: usize,
    pub graceful_closed_count: usize,
    pub force_killed_count: usize,
    pub skipped_count: usize,
    pub failed_count: usize,
    pub results: Vec<ProcessReleaseResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessReleaseResult {
    pub pid: Option<u32>,
    pub name: String,
    pub status: ProcessReleaseStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProcessReleaseStatus {
    WouldGracefulClose,
    WouldGracefulCloseThenForceIfNeeded,
    GracefulClosed,
    GracefulCloseRequested,
    ForceKilled,
    Skipped(ProcessReleaseSkipReason),
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProcessReleaseSkipReason {
    BlockedSystemOrServiceProcess,
    SystemBoundaryProcess,
    SelfProcess,
    ForceKillDisabled,
}

pub fn run_process_release_all(
    dry_run: bool,
    allow_force_kill: bool,
) -> Result<ProcessReleaseReport> {
    run_process_release(dry_run, allow_force_kill, None, usize::MAX, false)
}

pub fn run_targeted_process_release(
    dry_run: bool,
    allow_force_kill: bool,
    target_memory_load_percent: u32,
    max_terminated_processes: usize,
) -> Result<ProcessReleaseReport> {
    run_process_release(
        dry_run,
        allow_force_kill,
        Some(target_memory_load_percent),
        max_terminated_processes,
        true,
    )
}

fn run_process_release(
    dry_run: bool,
    allow_force_kill: bool,
    target_memory_load_percent: Option<u32>,
    max_terminated_processes: usize,
    keep_vm_stack: bool,
) -> Result<ProcessReleaseReport> {
    let started = Instant::now();
    let mut report = ProcessReleaseReport {
        dry_run,
        enabled: true,
        allow_force_kill,
        duration_ms: 0,
        scanned_count: 0,
        matched_count: 0,
        graceful_closed_count: 0,
        force_killed_count: 0,
        skipped_count: 0,
        failed_count: 0,
        results: Vec::new(),
    };

    run_process_release_pass(
        &mut report,
        dry_run,
        allow_force_kill,
        target_memory_load_percent,
        max_terminated_processes,
        keep_vm_stack,
    )?;

    if !dry_run && allow_force_kill {
        for _ in 0..2 {
            std::thread::sleep(Duration::from_millis(500));
            let before = report.force_killed_count + report.graceful_closed_count;
            run_process_release_pass(
                &mut report,
                false,
                true,
                target_memory_load_percent,
                max_terminated_processes,
                keep_vm_stack,
            )?;
            let after = report.force_killed_count + report.graceful_closed_count;
            if after == before || target_reached(target_memory_load_percent) {
                break;
            }
        }
    }

    report.duration_ms = started.elapsed().as_millis() as u64;
    Ok(report)
}

fn run_process_release_pass(
    report: &mut ProcessReleaseReport,
    dry_run: bool,
    allow_force_kill: bool,
    target_memory_load_percent: Option<u32>,
    max_terminated_processes: usize,
    keep_vm_stack: bool,
) -> Result<()> {
    let mut processes = enumerate_processes()?;
    processes.sort_by_key(|process| {
        std::cmp::Reverse((browser_priority(&process.name), process.working_set))
    });
    report.scanned_count += processes.len();

    for process in processes {
        if target_reached(target_memory_load_percent) {
            break;
        }
        if !dry_run
            && report.force_killed_count + report.graceful_closed_count >= max_terminated_processes
        {
            break;
        }
        if process.is_self {
            push_skipped(
                report,
                process.pid,
                process.name,
                ProcessReleaseSkipReason::SelfProcess,
            );
            continue;
        }
        if process.pid <= 4 || is_system_boundary_process(&process.name) {
            push_skipped(
                report,
                process.pid,
                process.name,
                ProcessReleaseSkipReason::SystemBoundaryProcess,
            );
            continue;
        }
        if is_process_release_blocked(&process.name, keep_vm_stack) {
            push_skipped(
                report,
                process.pid,
                process.name,
                ProcessReleaseSkipReason::BlockedSystemOrServiceProcess,
            );
            continue;
        }

        report.matched_count += 1;
        if dry_run {
            report.results.push(ProcessReleaseResult {
                pid: Some(process.pid),
                name: process.name,
                status: if allow_force_kill {
                    ProcessReleaseStatus::WouldGracefulCloseThenForceIfNeeded
                } else {
                    ProcessReleaseStatus::WouldGracefulClose
                },
            });
            continue;
        }

        match close_or_terminate(process.pid, &process.name, allow_force_kill) {
            ProcessReleaseStatus::GracefulClosed => {
                report.graceful_closed_count += 1;
                report.results.push(ProcessReleaseResult {
                    pid: Some(process.pid),
                    name: process.name,
                    status: ProcessReleaseStatus::GracefulClosed,
                });
            }
            ProcessReleaseStatus::ForceKilled => {
                report.force_killed_count += 1;
                report.results.push(ProcessReleaseResult {
                    pid: Some(process.pid),
                    name: process.name,
                    status: ProcessReleaseStatus::ForceKilled,
                });
            }
            ProcessReleaseStatus::GracefulCloseRequested => {
                report.results.push(ProcessReleaseResult {
                    pid: Some(process.pid),
                    name: process.name,
                    status: ProcessReleaseStatus::GracefulCloseRequested,
                });
            }
            ProcessReleaseStatus::Skipped(reason) => {
                report.skipped_count += 1;
                report.results.push(ProcessReleaseResult {
                    pid: Some(process.pid),
                    name: process.name,
                    status: ProcessReleaseStatus::Skipped(reason),
                });
            }
            ProcessReleaseStatus::Failed(message) => {
                report.failed_count += 1;
                report.results.push(ProcessReleaseResult {
                    pid: Some(process.pid),
                    name: process.name,
                    status: ProcessReleaseStatus::Failed(message),
                });
            }
            other => {
                report.failed_count += 1;
                report.results.push(ProcessReleaseResult {
                    pid: Some(process.pid),
                    name: process.name,
                    status: ProcessReleaseStatus::Failed(format!(
                        "unexpected process release status: {other:?}"
                    )),
                });
            }
        }
    }

    Ok(())
}

fn target_reached(target_memory_load_percent: Option<u32>) -> bool {
    let Some(target) = target_memory_load_percent else {
        return false;
    };
    match crate::memory::get_memory_snapshot() {
        Ok(snapshot) => snapshot.memory_load_percent <= target,
        Err(_) => false,
    }
}

fn browser_priority(process_name: &str) -> u8 {
    if browser_processes()
        .iter()
        .any(|browser| same_process_name(browser, process_name))
    {
        1
    } else {
        0
    }
}

fn close_or_terminate(pid: u32, _name: &str, allow_force_kill: bool) -> ProcessReleaseStatus {
    match winapi::graceful_close_process_windows(pid) {
        Ok(posted) if posted > 0 => match winapi::wait_for_process_exit(pid, 5000) {
            Ok(true) => ProcessReleaseStatus::GracefulClosed,
            Ok(false) if allow_force_kill => force_kill(pid),
            Ok(false) => ProcessReleaseStatus::GracefulCloseRequested,
            Err(err) if err.is_process_exited() => ProcessReleaseStatus::GracefulClosed,
            Err(err) => ProcessReleaseStatus::Failed(err.to_string()),
        },
        Ok(_) if allow_force_kill => force_kill(pid),
        Ok(_) => ProcessReleaseStatus::Skipped(ProcessReleaseSkipReason::ForceKillDisabled),
        Err(err) if err.is_process_exited() => ProcessReleaseStatus::GracefulClosed,
        Err(_) if allow_force_kill => force_kill(pid),
        Err(err) => ProcessReleaseStatus::Failed(err.to_string()),
    }
}

fn force_kill(pid: u32) -> ProcessReleaseStatus {
    match winapi::terminate_process(pid) {
        Ok(()) => match winapi::wait_for_process_exit(pid, 3000) {
            Ok(true) => ProcessReleaseStatus::ForceKilled,
            Ok(false) => ProcessReleaseStatus::Failed(
                "强制终止已请求，但进程在超时时间内仍未退出。".to_owned(),
            ),
            Err(err) if err.is_process_exited() => ProcessReleaseStatus::ForceKilled,
            Err(err) => ProcessReleaseStatus::Failed(err.to_string()),
        },
        Err(err) => ProcessReleaseStatus::Failed(err.to_string()),
    }
}

fn push_skipped(
    report: &mut ProcessReleaseReport,
    pid: u32,
    name: String,
    reason: ProcessReleaseSkipReason,
) {
    report.skipped_count += 1;
    report.results.push(ProcessReleaseResult {
        pid: (pid != 0).then_some(pid),
        name,
        status: ProcessReleaseStatus::Skipped(reason),
    });
}

fn is_system_boundary_process(name: &str) -> bool {
    system_boundary_processes()
        .iter()
        .any(|fixed| same_process_name(fixed, name))
}

fn is_process_release_blocked(name: &str, keep_vm_stack: bool) -> bool {
    process_release_blocked_processes()
        .iter()
        .any(|blocked| same_process_name(blocked, name))
        || is_system_boundary_process(name)
        || (keep_vm_stack
            && vm_stack_processes()
                .iter()
                .any(|blocked| same_process_name(blocked, name)))
}

fn process_release_blocked_processes() -> &'static [&'static str] {
    &[
        "System",
        "Idle",
        "Registry",
        "smss.exe",
        "csrss.exe",
        "wininit.exe",
        "services.exe",
        "lsass.exe",
        "winlogon.exe",
        "fontdrvhost.exe",
        "dwm.exe",
        "explorer.exe",
        "svchost.exe",
        "conhost.exe",
        "sihost.exe",
        "RuntimeBroker.exe",
        "SearchIndexer.exe",
        "audiodg.exe",
        "spoolsv.exe",
        "memspark.exe",
        "memspark-ui.exe",
    ]
}
