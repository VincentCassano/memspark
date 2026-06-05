use crate::memory::{get_memory_snapshot, MemorySnapshot};
use crate::termination::run_targeted_process_release;
use crate::winapi::{self, MemoryListCommand};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const MEMORY_OPTIMIZATION_TARGET_MEMORY_LOAD_PERCENT: u32 = 25;
const MEMORY_OPTIMIZATION_SYSTEM_ROUNDS: u32 = 12;
const MEMORY_OPTIMIZATION_TERMINATION_SWEEPS: u32 = 8;
const MIN_MEANINGFUL_AVAILABLE_GAIN_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemReleaseReport {
    pub dry_run: bool,
    pub duration_ms: u64,
    pub before: MemorySnapshot,
    pub after: MemorySnapshot,
    pub steps: Vec<SystemReleaseStepResult>,
    pub failed_count: usize,
    pub target_memory_load_percent: u32,
    pub target_reached: bool,
    #[serde(default)]
    pub termination_matched_count: usize,
    #[serde(default)]
    pub termination_graceful_closed_count: usize,
    #[serde(default)]
    pub termination_force_killed_count: usize,
    #[serde(default)]
    pub termination_failed_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemReleaseStepResult {
    pub step: SystemReleaseStep,
    pub status: SystemReleaseStepStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SystemReleaseStep {
    EnableDebugPrivilege,
    EnableIncreaseQuotaPrivilege,
    EnableProfilePrivilege,
    EmptySystemWorkingSets,
    FlushModifiedPageList,
    PurgeLowPriorityStandbyList,
    PurgeStandbyList,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemReleaseStepStatus {
    WouldRun,
    Succeeded,
    Failed(String),
}

impl SystemReleaseReport {
    pub fn available_increase(&self) -> i64 {
        self.after.available_physical as i64 - self.before.available_physical as i64
    }
}

pub fn run_system_release(dry_run: bool) -> Result<SystemReleaseReport> {
    let target = MEMORY_OPTIMIZATION_TARGET_MEMORY_LOAD_PERCENT;
    let mut report =
        run_targeted_system_release(dry_run, target, MEMORY_OPTIMIZATION_SYSTEM_ROUNDS)?;
    if !dry_run && !report.target_reached {
        for _ in 0..MEMORY_OPTIMIZATION_TERMINATION_SWEEPS {
            let termination = run_targeted_process_release(false, true, target, usize::MAX)?;
            report.termination_matched_count += termination.matched_count;
            report.termination_graceful_closed_count += termination.graceful_closed_count;
            report.termination_force_killed_count += termination.force_killed_count;
            report.termination_failed_count += termination.failed_count;
            report = merge_release_pass(
                report,
                run_targeted_system_release(false, target, MEMORY_OPTIMIZATION_SYSTEM_ROUNDS / 2)?,
            );

            if report.target_reached {
                break;
            }

            if termination.graceful_closed_count + termination.force_killed_count == 0 {
                break;
            }
        }
    }
    Ok(report)
}

pub fn run_targeted_system_release(
    dry_run: bool,
    target_memory_load_percent: u32,
    max_rounds: u32,
) -> Result<SystemReleaseReport> {
    let started = Instant::now();
    let before = get_memory_snapshot()?;
    let mut steps = Vec::new();
    let rounds = if dry_run { 1 } else { max_rounds.max(1) };

    let debug_ready = run_step(
        &mut steps,
        dry_run,
        SystemReleaseStep::EnableDebugPrivilege,
        winapi::enable_debug_privilege,
    );
    let quota_ready = run_step(
        &mut steps,
        dry_run,
        SystemReleaseStep::EnableIncreaseQuotaPrivilege,
        winapi::enable_increase_quota_privilege,
    );
    let profile_ready = run_step(
        &mut steps,
        dry_run,
        SystemReleaseStep::EnableProfilePrivilege,
        winapi::enable_profile_privilege,
    );

    let mut after = before.clone();
    if profile_ready || debug_ready || quota_ready || dry_run {
        let mut stagnant_rounds = 0;
        for round in 0..rounds {
            let round_before_available = after.available_physical;
            let mut round_succeeded = false;

            for step in release_plan(after.memory_load_percent, target_memory_load_percent, round) {
                round_succeeded |= run_memory_list_step(&mut steps, dry_run, step);

                if dry_run {
                    continue;
                }

                std::thread::sleep(Duration::from_millis(release_step_delay_ms(step, round)));
                after = get_memory_snapshot().unwrap_or_else(|_| after.clone());
                if after.memory_load_percent <= target_memory_load_percent {
                    break;
                }
            }

            if dry_run {
                break;
            }

            if after.memory_load_percent <= target_memory_load_percent || !round_succeeded {
                break;
            }

            let gained = after
                .available_physical
                .saturating_sub(round_before_available);
            if gained < MIN_MEANINGFUL_AVAILABLE_GAIN_BYTES {
                stagnant_rounds += 1;
            } else {
                stagnant_rounds = 0;
            }

            if stagnant_rounds >= 2 || round + 1 >= rounds {
                break;
            }
        }
    }

    if !dry_run {
        after = get_memory_snapshot().unwrap_or(after);
    }
    let failed_count = steps
        .iter()
        .filter(|step| matches!(step.status, SystemReleaseStepStatus::Failed(_)))
        .count();
    let target_reached = !dry_run && after.memory_load_percent <= target_memory_load_percent;

    Ok(SystemReleaseReport {
        dry_run,
        duration_ms: started.elapsed().as_millis() as u64,
        before,
        after,
        steps,
        failed_count,
        target_memory_load_percent,
        target_reached,
        termination_matched_count: 0,
        termination_graceful_closed_count: 0,
        termination_force_killed_count: 0,
        termination_failed_count: 0,
    })
}

fn run_step<F>(
    steps: &mut Vec<SystemReleaseStepResult>,
    dry_run: bool,
    step: SystemReleaseStep,
    action: F,
) -> bool
where
    F: FnOnce() -> std::result::Result<(), winapi::WinApiError>,
{
    if dry_run {
        steps.push(SystemReleaseStepResult {
            step,
            status: SystemReleaseStepStatus::WouldRun,
        });
        return true;
    }

    let status = match action() {
        Ok(()) => SystemReleaseStepStatus::Succeeded,
        Err(err) => SystemReleaseStepStatus::Failed(err.to_string()),
    };
    let succeeded = matches!(status, SystemReleaseStepStatus::Succeeded);
    steps.push(SystemReleaseStepResult { step, status });
    succeeded
}

fn run_memory_list_step(
    steps: &mut Vec<SystemReleaseStepResult>,
    dry_run: bool,
    step: SystemReleaseStep,
) -> bool {
    run_step(steps, dry_run, step, || match step {
        SystemReleaseStep::EmptySystemWorkingSets => {
            winapi::set_system_memory_list(MemoryListCommand::EmptyWorkingSets)
        }
        SystemReleaseStep::FlushModifiedPageList => {
            winapi::set_system_memory_list(MemoryListCommand::FlushModifiedList)
        }
        SystemReleaseStep::PurgeLowPriorityStandbyList => {
            winapi::set_system_memory_list(MemoryListCommand::PurgeLowPriorityStandbyList)
        }
        SystemReleaseStep::PurgeStandbyList => {
            winapi::set_system_memory_list(MemoryListCommand::PurgeStandbyList)
        }
        SystemReleaseStep::EnableDebugPrivilege
        | SystemReleaseStep::EnableIncreaseQuotaPrivilege
        | SystemReleaseStep::EnableProfilePrivilege => Ok(()),
    })
}

fn release_plan(
    memory_load_percent: u32,
    target_memory_load_percent: u32,
    round: u32,
) -> Vec<SystemReleaseStep> {
    let pressure = memory_load_percent.saturating_sub(target_memory_load_percent);
    if pressure >= 35 {
        if round % 2 == 0 {
            vec![
                SystemReleaseStep::PurgeStandbyList,
                SystemReleaseStep::EmptySystemWorkingSets,
                SystemReleaseStep::FlushModifiedPageList,
                SystemReleaseStep::PurgeLowPriorityStandbyList,
            ]
        } else {
            vec![
                SystemReleaseStep::EmptySystemWorkingSets,
                SystemReleaseStep::PurgeStandbyList,
                SystemReleaseStep::PurgeLowPriorityStandbyList,
                SystemReleaseStep::FlushModifiedPageList,
            ]
        }
    } else if pressure >= 15 {
        if round % 2 == 0 {
            vec![
                SystemReleaseStep::PurgeLowPriorityStandbyList,
                SystemReleaseStep::EmptySystemWorkingSets,
                SystemReleaseStep::FlushModifiedPageList,
            ]
        } else {
            vec![
                SystemReleaseStep::EmptySystemWorkingSets,
                SystemReleaseStep::PurgeStandbyList,
            ]
        }
    } else if round % 2 == 0 {
        vec![
            SystemReleaseStep::PurgeLowPriorityStandbyList,
            SystemReleaseStep::EmptySystemWorkingSets,
        ]
    } else {
        vec![
            SystemReleaseStep::FlushModifiedPageList,
            SystemReleaseStep::PurgeLowPriorityStandbyList,
        ]
    }
}

fn release_step_delay_ms(step: SystemReleaseStep, round: u32) -> u64 {
    let base = match step {
        SystemReleaseStep::EmptySystemWorkingSets => 110,
        SystemReleaseStep::FlushModifiedPageList => 160,
        SystemReleaseStep::PurgeLowPriorityStandbyList => 120,
        SystemReleaseStep::PurgeStandbyList => 190,
        SystemReleaseStep::EnableDebugPrivilege
        | SystemReleaseStep::EnableIncreaseQuotaPrivilege
        | SystemReleaseStep::EnableProfilePrivilege => 0,
    };
    base + u64::from(round.min(4)) * 25
}

fn merge_release_pass(
    mut first: SystemReleaseReport,
    mut second: SystemReleaseReport,
) -> SystemReleaseReport {
    second.before = first.before;
    first.steps.extend(second.steps);
    second.steps = first.steps;
    second.failed_count += first.failed_count;
    second.termination_matched_count += first.termination_matched_count;
    second.termination_graceful_closed_count += first.termination_graceful_closed_count;
    second.termination_force_killed_count += first.termination_force_killed_count;
    second.termination_failed_count += first.termination_failed_count;
    second
}
