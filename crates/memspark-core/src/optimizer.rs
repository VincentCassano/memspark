use crate::config::AppConfig;
use crate::memory::{get_memory_snapshot, MemorySnapshot};
use crate::policy::{
    same_process_name, system_boundary_processes, ModePolicy, OptimizeMode, SkipReason, TrimStatus,
};
use crate::process::{enumerate_processes, ProcessInfo};
use crate::release::run_system_release;
use crate::report::{CleanupEffectSummary, OptimizeReport, TrimResult};
use crate::winapi::{self, MemoryListCommand};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MEMORY_OPTIMIZATION_TARGET_PERCENT: u32 = 25;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggressiveTrimOptions {
    pub min_working_set_bytes: u64,
    pub rounds: u32,
    pub round_delay_ms: u64,
    pub allow_trim_explorer: bool,
    pub allow_trim_browser: bool,
    pub include_query_failed: bool,
    pub skip_foreground: bool,
    pub skip_system: bool,
    pub skip_self: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggressiveTrimReport {
    pub results: Vec<TrimResult>,
    pub process_trim_delta_bytes: u64,
    pub rounds_executed: u32,
}

#[derive(Debug, Clone)]
struct CandidateProcess {
    pid: u32,
    current_working_set: u64,
    result: TrimResult,
}

#[derive(Debug, Clone)]
struct SystemCleanupOutcome {
    executed: bool,
    skipped_reason: Option<String>,
    delta_bytes: Option<i64>,
}

#[derive(Debug, Clone, Default)]
struct TerminationTotals {
    matched_count: usize,
    closed_count: usize,
    force_killed_count: usize,
    failed_count: usize,
}

pub fn optimize(dry_run: bool, config: &AppConfig) -> Result<OptimizeReport> {
    let started = Instant::now();
    let before = get_memory_snapshot()?;
    let policy = config.policy().clone();
    let options = trim_options(&policy);
    if !dry_run {
        let _ = winapi::enable_debug_privilege();
        let _ = winapi::enable_increase_quota_privilege();
        let _ = winapi::enable_profile_privilege();
    }
    let trim_report = trim_working_sets_aggressive(options, dry_run)?;

    let system_cleanup = run_memory_optimization_target_cleanup(dry_run);

    let after = get_memory_snapshot().unwrap_or_else(|_| before.clone());
    let scanned_count = trim_report.results.len();
    let trimmed_count = trim_report
        .results
        .iter()
        .filter(|result| matches!(result.status, TrimStatus::Trimmed | TrimStatus::WouldTrim))
        .count();
    let skipped_count = trim_report
        .results
        .iter()
        .filter(|result| matches!(result.status, TrimStatus::Skipped(_)))
        .count();
    let failed_count = trim_report
        .results
        .iter()
        .filter(|result| matches!(result.status, TrimStatus::Failed(_)))
        .count();
    let effect = cleanup_effect_summary(&before, &after, &trim_report, &system_cleanup);

    Ok(OptimizeReport {
        mode: OptimizeMode::MemoryOptimization,
        dry_run,
        timestamp_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        duration_ms: started.elapsed().as_millis() as u64,
        before,
        after,
        scanned_count,
        trimmed_count,
        skipped_count,
        failed_count,
        effect,
        results: trim_report.results,
    })
}

pub fn trim_working_sets_aggressive(
    options: AggressiveTrimOptions,
    dry_run: bool,
) -> Result<AggressiveTrimReport> {
    let rounds = if dry_run { 1 } else { options.rounds.max(1) };
    let mut processes = enumerate_processes()?;
    processes.sort_by_key(|process| std::cmp::Reverse(process.working_set));

    let mut results = Vec::with_capacity(processes.len());
    let mut candidates = Vec::new();

    for process in processes {
        if let Some(skip_reason) = skip_reason(&process, &options) {
            results.push(TrimResult {
                pid: process.pid,
                name: process.name,
                before_working_set: process.working_set,
                after_working_set: None,
                status: TrimStatus::Skipped(skip_reason),
            });
            continue;
        }

        let result = TrimResult {
            pid: process.pid,
            name: process.name,
            before_working_set: process.working_set,
            after_working_set: None,
            status: if dry_run {
                TrimStatus::WouldTrim
            } else {
                TrimStatus::Trimmed
            },
        };

        candidates.push(CandidateProcess {
            pid: process.pid,
            current_working_set: process.working_set.max(options.min_working_set_bytes),
            result,
        });
    }

    if dry_run {
        results.extend(candidates.into_iter().map(|candidate| candidate.result));
        return Ok(AggressiveTrimReport {
            results,
            process_trim_delta_bytes: 0,
            rounds_executed: options.rounds.max(1),
        });
    }

    let mut rounds_executed = 0;
    for round in 0..rounds {
        let mut touched_this_round = false;

        for candidate in &mut candidates {
            if !matches!(candidate.result.status, TrimStatus::Trimmed) {
                continue;
            }
            if candidate.current_working_set < options.min_working_set_bytes {
                continue;
            }

            touched_this_round = true;
            match trim_one_process(candidate.pid) {
                TrimStatus::Trimmed => {
                    if let Ok(after) = winapi::query_process_working_set(candidate.pid) {
                        candidate.current_working_set = after;
                        candidate.result.after_working_set = Some(after);
                    }
                }
                status @ TrimStatus::Skipped(_) | status @ TrimStatus::Failed(_) => {
                    if round == 0 {
                        candidate.result.status = status;
                    }
                }
                TrimStatus::WouldTrim => {}
            }
        }

        if touched_this_round {
            rounds_executed = round + 1;
        }

        if !touched_this_round || round + 1 >= rounds {
            break;
        }

        std::thread::sleep(Duration::from_millis(options.round_delay_ms));
    }
    if rounds_executed == 0
        && candidates.iter().any(|candidate| {
            matches!(
                candidate.result.status,
                TrimStatus::Trimmed | TrimStatus::Skipped(_) | TrimStatus::Failed(_)
            )
        })
    {
        rounds_executed = 1;
    }

    let process_trim_delta_bytes = candidates
        .iter()
        .filter(|candidate| matches!(candidate.result.status, TrimStatus::Trimmed))
        .filter_map(|candidate| {
            candidate
                .result
                .after_working_set
                .map(|after| (candidate, after))
        })
        .map(|(candidate, after)| candidate.result.before_working_set.saturating_sub(after))
        .sum();

    results.extend(candidates.into_iter().map(|candidate| candidate.result));

    Ok(AggressiveTrimReport {
        results,
        process_trim_delta_bytes,
        rounds_executed,
    })
}

fn trim_options(policy: &ModePolicy) -> AggressiveTrimOptions {
    AggressiveTrimOptions {
        min_working_set_bytes: policy.min_working_set_bytes(),
        rounds: policy.max_rounds.max(1),
        round_delay_ms: policy.round_delay_ms,
        allow_trim_explorer: policy.allow_trim_explorer,
        allow_trim_browser: policy.allow_trim_browser,
        include_query_failed: false,
        skip_foreground: policy.skip_foreground,
        skip_system: policy.skip_system_process,
        skip_self: policy.skip_self,
    }
    .normalized_for_memory_optimization()
}

impl AggressiveTrimOptions {
    fn normalized_for_memory_optimization(mut self) -> Self {
        self.rounds = self.rounds.max(5);
        self.include_query_failed = true;
        if !(100..=300).contains(&self.round_delay_ms) {
            self.round_delay_ms = 150;
        }
        self
    }
}

fn skip_reason(process: &ProcessInfo, options: &AggressiveTrimOptions) -> Option<SkipReason> {
    if options.skip_system && is_internal_system_boundary(&process.name) {
        return Some(SkipReason::SystemBoundaryProcess);
    }

    if options.skip_system && process.pid <= 4 {
        return Some(SkipReason::SystemProcess);
    }

    if options.skip_self && process.is_self {
        return Some(SkipReason::SelfProcess);
    }

    if options.skip_foreground && process.is_foreground {
        return Some(SkipReason::ForegroundProcess);
    }

    if process.query_error.is_some() && !options.include_query_failed {
        return Some(SkipReason::QueryFailed);
    }

    if process.working_set < options.min_working_set_bytes && process.query_error.is_none() {
        return Some(SkipReason::WorkingSetTooSmall);
    }

    None
}

fn trim_one_process(pid: u32) -> TrimStatus {
    match winapi::trim_process_empty_working_set(pid)
        .or_else(|_| winapi::trim_process_set_working_set_size_ex(pid))
    {
        Ok(()) => TrimStatus::Trimmed,
        Err(err) if err.is_access_denied() => TrimStatus::Skipped(SkipReason::AccessDenied),
        Err(err) if err.is_process_exited() => TrimStatus::Skipped(SkipReason::ProcessExited),
        Err(err) if err.context.starts_with("OpenProcess") => {
            TrimStatus::Skipped(SkipReason::OpenProcessFailed)
        }
        Err(err) => TrimStatus::Failed(err.to_string()),
    }
}

fn is_internal_system_boundary(process_name: &str) -> bool {
    system_boundary_processes()
        .iter()
        .any(|fixed| same_process_name(fixed, process_name))
}

fn run_memory_optimization_target_cleanup(dry_run: bool) -> SystemCleanupOutcome {
    match run_system_release(dry_run) {
        Ok(report) => {
            let termination_totals = TerminationTotals {
                matched_count: report.termination_matched_count,
                closed_count: report.termination_graceful_closed_count,
                force_killed_count: report.termination_force_killed_count,
                failed_count: report.termination_failed_count,
            };
            let skipped_reason = if dry_run {
                Some(format!(
                    "dry-run 仅预览内存优化步骤；实际执行目标为内存占用不高于 {}%",
                    MEMORY_OPTIMIZATION_TARGET_PERCENT
                ))
            } else if report.target_reached {
                termination_activity_message(&termination_totals)
            } else {
                Some(format!(
                    "内存优化已执行全部可用释放步骤，但当前内存占用仍为 {}%，未达到 {}% 目标；剩余占用可能来自系统保留、驱动、内核或不可终止进程",
                    report.after.memory_load_percent, report.target_memory_load_percent
                ))
            };
            let skipped_reason = append_termination_activity(skipped_reason, &termination_totals);

            SystemCleanupOutcome {
                executed: !dry_run
                    && (report.available_increase() != 0
                        || termination_totals.closed_count > 0
                        || termination_totals.force_killed_count > 0),
                skipped_reason,
                delta_bytes: (!dry_run).then(|| report.available_increase()),
            }
        }
        Err(err) => SystemCleanupOutcome {
            executed: false,
            skipped_reason: Some(format!("内存优化目标释放失败：{err}")),
            delta_bytes: None,
        },
    }
}

#[allow(dead_code)]
fn run_optional_system_cleanup(config: &AppConfig, dry_run: bool) -> SystemCleanupOutcome {
    let cleanup = &config.advanced_cleanup;
    let any_configured = cleanup.enable_standby_list_cleanup
        || cleanup.enable_system_working_set_cleanup
        || cleanup.enable_modified_page_list_cleanup;

    if !any_configured {
        return SystemCleanupOutcome {
            executed: false,
            skipped_reason: Some("advanced_cleanup 未启用，已跳过系统级清理".to_owned()),
            delta_bytes: None,
        };
    }

    if dry_run {
        return SystemCleanupOutcome {
            executed: false,
            skipped_reason: Some("dry-run 只预览，不执行系统级清理".to_owned()),
            delta_bytes: None,
        };
    }

    if cleanup.require_admin_for_system_cleanup && !winapi::is_running_as_admin() {
        return SystemCleanupOutcome {
            executed: false,
            skipped_reason: Some("需要管理员权限，已跳过 standby list / 系统工作集清理".to_owned()),
            delta_bytes: None,
        };
    }

    let before = get_memory_snapshot().ok();
    let mut executed = false;
    let mut messages = Vec::new();

    if let Err(err) = winapi::enable_profile_privilege() {
        return SystemCleanupOutcome {
            executed: false,
            skipped_reason: Some(format!("启用系统级清理权限失败：{err}")),
            delta_bytes: None,
        };
    }

    if cleanup.enable_system_working_set_cleanup {
        match winapi::set_system_memory_list(MemoryListCommand::EmptyWorkingSets) {
            Ok(()) => executed = true,
            Err(err) => messages.push(format!("系统工作集清理失败：{err}")),
        }
    }

    if cleanup.enable_standby_list_cleanup {
        match winapi::set_system_memory_list(MemoryListCommand::PurgeLowPriorityStandbyList) {
            Ok(()) => executed = true,
            Err(err) => messages.push(format!("低优先级 standby list 清理失败：{err}")),
        }
        match winapi::set_system_memory_list(MemoryListCommand::PurgeStandbyList) {
            Ok(()) => executed = true,
            Err(err) => messages.push(format!("standby list 清理失败：{err}")),
        }
    }

    if cleanup.enable_modified_page_list_cleanup {
        messages.push("modified page list 清理已预留，本版本默认不执行".to_owned());
    }

    let delta_bytes = if executed {
        match (before, get_memory_snapshot().ok()) {
            (Some(before), Some(after)) => {
                Some(after.available_physical as i64 - before.available_physical as i64)
            }
            _ => None,
        }
    } else {
        None
    };

    SystemCleanupOutcome {
        executed,
        skipped_reason: (!messages.is_empty()).then(|| messages.join("; ")),
        delta_bytes,
    }
}

fn cleanup_effect_summary(
    before: &MemorySnapshot,
    after: &MemorySnapshot,
    trim_report: &AggressiveTrimReport,
    system_cleanup: &SystemCleanupOutcome,
) -> CleanupEffectSummary {
    CleanupEffectSummary {
        before_physical_used_bytes: before.used_physical(),
        after_physical_used_bytes: after.used_physical(),
        before_available_bytes: before.available_physical,
        after_available_bytes: after.available_physical,
        available_delta_bytes: after.available_physical as i64 - before.available_physical as i64,
        process_trim_delta_bytes: trim_report.process_trim_delta_bytes,
        standby_cleanup_delta_bytes: system_cleanup.delta_bytes,
        system_cleanup_executed: system_cleanup.executed,
        system_cleanup_skipped_reason: system_cleanup.skipped_reason.clone(),
        working_set_rounds: trim_report.rounds_executed,
    }
}

fn termination_activity_message(totals: &TerminationTotals) -> Option<String> {
    let terminated = totals.closed_count + totals.force_killed_count;
    if totals.matched_count == 0 && terminated == 0 && totals.failed_count == 0 {
        None
    } else {
        Some(format!(
            "目标释放已进入终止阶段：候选 {}，温和关闭 {}，强制终止 {}，失败 {}",
            totals.matched_count,
            totals.closed_count,
            totals.force_killed_count,
            totals.failed_count
        ))
    }
}

fn append_termination_activity(
    reason: Option<String>,
    totals: &TerminationTotals,
) -> Option<String> {
    match (reason, termination_activity_message(totals)) {
        (Some(reason), Some(activity)) if !reason.contains(&activity) => {
            Some(format!("{reason}; {activity}"))
        }
        (Some(reason), _) => Some(reason),
        (None, activity) => activity,
    }
}
