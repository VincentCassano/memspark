use crate::config::{
    default_developer_log_dir, default_report_history_dir, expand_env_path, AppConfig,
};
use crate::memory::MemorySnapshot;
use crate::policy::{OptimizeMode, SkipReason, TrimStatus};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const TARGET_MEMORY_LOAD_PERCENT: f64 = 25.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrimResult {
    pub pid: u32,
    pub name: String,
    pub before_working_set: u64,
    pub after_working_set: Option<u64>,
    pub status: TrimStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizeReport {
    pub mode: OptimizeMode,
    pub dry_run: bool,
    pub timestamp_unix: u64,
    pub duration_ms: u64,
    pub before: MemorySnapshot,
    pub after: MemorySnapshot,
    pub scanned_count: usize,
    pub trimmed_count: usize,
    pub skipped_count: usize,
    pub failed_count: usize,
    #[serde(default)]
    pub effect: CleanupEffectSummary,
    pub results: Vec<TrimResult>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CleanupEffectSummary {
    pub before_physical_used_bytes: u64,
    pub after_physical_used_bytes: u64,
    pub before_available_bytes: u64,
    pub after_available_bytes: u64,
    pub available_delta_bytes: i64,
    pub process_trim_delta_bytes: u64,
    pub standby_cleanup_delta_bytes: Option<i64>,
    pub system_cleanup_executed: bool,
    pub system_cleanup_skipped_reason: Option<String>,
    pub working_set_rounds: u32,
}

impl OptimizeReport {
    pub fn available_increase(&self) -> i64 {
        self.after.available_physical as i64 - self.before.available_physical as i64
    }

    pub fn skip_reason_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for result in &self.results {
            if let TrimStatus::Skipped(reason) = &result.status {
                *counts.entry(reason.to_string()).or_insert(0) += 1;
            }
        }
        counts
    }

    pub fn failed_reason_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for result in &self.results {
            if let TrimStatus::Failed(reason) = &result.status {
                *counts.entry(reason.clone()).or_insert(0) += 1;
            }
        }
        counts
    }
}

pub fn save_report(config: &AppConfig, report: &OptimizeReport) -> Result<Option<PathBuf>> {
    if !config.report.save_last_report {
        return Ok(None);
    }
    let path = expand_env_path(&config.report.last_report_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    archive_existing_report(config, &path)?;
    let content = serde_json::to_string_pretty(report)?;
    fs::write(&path, content)?;
    Ok(Some(path))
}

pub fn clear_report_history(config: &AppConfig) -> Result<usize> {
    let history_dir = report_history_dir(config);
    if !history_dir.exists() {
        return Ok(0);
    }

    let mut removed = 0usize;
    for entry in fs::read_dir(&history_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && is_json_file(&path) {
            fs::remove_file(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn save_developer_log(config: &AppConfig, report: &OptimizeReport) -> Result<Option<PathBuf>> {
    let log_dir = developer_log_dir(config);
    fs::create_dir_all(&log_dir)?;
    let path = unique_developer_log_path(&log_dir, report.timestamp_unix);
    fs::write(&path, format_developer_log(report)?)?;
    Ok(Some(path))
}

pub fn clear_developer_logs(config: &AppConfig) -> Result<usize> {
    let log_dir = developer_log_dir(config);
    if !log_dir.exists() {
        return Ok(0);
    }

    let mut removed = 0usize;
    for entry in fs::read_dir(&log_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && is_log_file(&path) {
            fs::remove_file(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn load_last_report(config: &AppConfig) -> Result<OptimizeReport> {
    let path = expand_env_path(&config.report.last_report_path);
    load_report_from_path(&path)
}

pub fn load_report_from_path(path: &Path) -> Result<OptimizeReport> {
    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

fn archive_existing_report(config: &AppConfig, last_report_path: &Path) -> Result<Option<PathBuf>> {
    if !last_report_path.exists() {
        return Ok(None);
    }

    let history_dir = report_history_dir(config);
    fs::create_dir_all(&history_dir)?;
    let timestamp = report_timestamp_from_path(last_report_path).unwrap_or_else(current_unix_time);
    let destination = unique_history_path(&history_dir, timestamp);
    fs::copy(last_report_path, &destination)?;
    Ok(Some(destination))
}

fn report_history_dir(config: &AppConfig) -> PathBuf {
    let configured = config.report.history_report_dir.trim();
    if configured.is_empty() {
        default_report_history_dir()
    } else {
        expand_env_path(configured)
    }
}

fn developer_log_dir(config: &AppConfig) -> PathBuf {
    let configured = config.report.developer_log_dir.trim();
    if configured.is_empty() {
        default_developer_log_dir()
    } else {
        expand_env_path(configured)
    }
}

fn report_timestamp_from_path(path: &Path) -> Option<u64> {
    let content = fs::read_to_string(path).ok()?;
    let report = serde_json::from_str::<OptimizeReport>(&content).ok()?;
    Some(report.timestamp_unix)
}

fn unique_history_path(history_dir: &Path, timestamp_unix: u64) -> PathBuf {
    let base = format!("report-{timestamp_unix}");
    for index in 0..1000 {
        let file_name = if index == 0 {
            format!("{base}.json")
        } else {
            format!("{base}-{index}.json")
        };
        let path = history_dir.join(file_name);
        if !path.exists() {
            return path;
        }
    }
    history_dir.join(format!("{base}-{}.json", current_unix_time()))
}

fn unique_developer_log_path(log_dir: &Path, timestamp_unix: u64) -> PathBuf {
    let base = format!("dev-report-{timestamp_unix}");
    for index in 0..1000 {
        let file_name = if index == 0 {
            format!("{base}.log")
        } else {
            format!("{base}-{index}.log")
        };
        let path = log_dir.join(file_name);
        if !path.exists() {
            return path;
        }
    }
    log_dir.join(format!("{base}-{}.log", current_unix_time()))
}

fn current_unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn is_json_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

fn is_log_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("log"))
}

fn format_developer_log(report: &OptimizeReport) -> Result<String> {
    let mut output = String::new();
    writeln!(output, "MemSpark Developer Log").ok();
    writeln!(output, "Timestamp Unix: {}", report.timestamp_unix).ok();
    writeln!(output, "Dry Run: {}", report.dry_run).ok();
    writeln!(output, "Duration Ms: {}", report.duration_ms).ok();
    writeln!(
        output,
        "Target: memory load <= {:.0}% ({})",
        TARGET_MEMORY_LOAD_PERCENT,
        if target_reached(report) {
            "reached"
        } else {
            "not reached"
        }
    )
    .ok();
    writeln!(
        output,
        "Memory Load: {:.1}% -> {:.1}%",
        memory_load_percent(&report.before),
        memory_load_percent(&report.after)
    )
    .ok();
    writeln!(output).ok();

    writeln!(output, "[Counters]").ok();
    writeln!(output, "Scanned: {}", report.scanned_count).ok();
    writeln!(output, "Handled Working Sets: {}", report.trimmed_count).ok();
    writeln!(output, "Kept/Skipped: {}", report.skipped_count).ok();
    writeln!(output, "Failed: {}", report.failed_count).ok();
    writeln!(output).ok();

    writeln!(output, "[Effect Summary]").ok();
    writeln!(
        output,
        "Before Physical Used: {}",
        format_gb(report.effect.before_physical_used_bytes)
    )
    .ok();
    writeln!(
        output,
        "After Physical Used: {}",
        format_gb(report.effect.after_physical_used_bytes)
    )
    .ok();
    writeln!(
        output,
        "Before Available: {}",
        format_gb(report.effect.before_available_bytes)
    )
    .ok();
    writeln!(
        output,
        "After Available: {}",
        format_gb(report.effect.after_available_bytes)
    )
    .ok();
    writeln!(
        output,
        "Available Delta: {}",
        format_signed_bytes(report.effect.available_delta_bytes)
    )
    .ok();
    writeln!(
        output,
        "Process Trim Estimate: {}",
        format_gb(report.effect.process_trim_delta_bytes)
    )
    .ok();
    writeln!(
        output,
        "System Cleanup Delta: {}",
        report
            .effect
            .standby_cleanup_delta_bytes
            .map(format_signed_bytes)
            .unwrap_or_else(|| "not recorded".to_owned())
    )
    .ok();
    writeln!(
        output,
        "System Cleanup Executed: {}",
        report.effect.system_cleanup_executed
    )
    .ok();
    writeln!(
        output,
        "System Cleanup Reason: {}",
        report
            .effect
            .system_cleanup_skipped_reason
            .as_deref()
            .unwrap_or("none")
    )
    .ok();
    writeln!(
        output,
        "Working Set Rounds: {}",
        report.effect.working_set_rounds
    )
    .ok();
    writeln!(output).ok();

    writeln!(output, "[Skip Reason Counts]").ok();
    write_counts(&mut output, report.skip_reason_counts());
    writeln!(output).ok();

    writeln!(output, "[Failed Reason Counts]").ok();
    write_counts(&mut output, report.failed_reason_counts());
    writeln!(output).ok();

    writeln!(output, "[Detailed Text Report]").ok();
    writeln!(output, "{}", format_report_text_with_options(report, true)).ok();
    writeln!(output).ok();

    writeln!(output, "[Process Results]").ok();
    writeln!(
        output,
        "pid\tname\tbefore_working_set\tafter_working_set\tstatus"
    )
    .ok();
    for result in &report.results {
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}",
            result.pid,
            result.name,
            result.before_working_set,
            result
                .after_working_set
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_owned()),
            result.status
        )
        .ok();
    }
    writeln!(output).ok();

    writeln!(output, "[Raw Report JSON]").ok();
    writeln!(output, "{}", serde_json::to_string_pretty(report)?).ok();
    Ok(output)
}

fn write_counts(output: &mut String, counts: BTreeMap<String, usize>) {
    if counts.is_empty() {
        writeln!(output, "none").ok();
    } else {
        for (reason, count) in counts {
            writeln!(output, "{reason}: {count}").ok();
        }
    }
}

pub fn format_report_text(report: &OptimizeReport) -> String {
    format_report_text_with_options(report, false)
}

pub fn format_report_text_with_options(report: &OptimizeReport, verbose: bool) -> String {
    if report.dry_run {
        return format_dry_run(report, verbose);
    }

    let mut output = String::new();
    output.push_str("MemSpark Report\n");
    output.push_str(&format!(
        "Target: memory load <= {:.0}% ({})\n",
        TARGET_MEMORY_LOAD_PERCENT,
        if target_reached(report) {
            "reached"
        } else {
            "not reached"
        }
    ));
    output.push_str(&format!(
        "Final memory load: {:.1}%\n",
        memory_load_percent(&report.after)
    ));
    output.push_str(&format!(
        "Duration: {:.2}s\n\n",
        report.duration_ms as f64 / 1000.0
    ));
    output.push_str("Before:\n");
    output.push_str(&format_snapshot(&report.before));
    output.push('\n');
    output.push_str("After:\n");
    output.push_str(&format_snapshot(&report.after));
    output.push('\n');
    output.push_str("Result:\n");
    output.push_str(&format!(
        "Available change:    {}\n",
        format_signed_bytes(report.available_increase())
    ));
    output.push_str(&format!(
        "Working-set handled: {} / {}\n",
        report.trimmed_count, report.scanned_count
    ));
    output.push_str(&format!("Kept/skipped:        {}\n", report.skipped_count));
    output.push_str(&format!("Failed:              {}\n", report.failed_count));
    output.push_str(&format!(
        "System release:      {}\n\n",
        system_release_value_text(report)
    ));

    output.push_str("Effect summary:\n");
    output.push_str(&format!(
        "Working-set rounds:  {}\n",
        report.effect.working_set_rounds.max(1)
    ));
    output.push_str(&format!(
        "Working-set estimate: {}\n",
        format_gb(report.effect.process_trim_delta_bytes)
    ));
    output.push_str(&format!(
        "System cleanup:      {}\n",
        if report.effect.system_cleanup_executed {
            "executed"
        } else {
            "skipped"
        }
    ));
    if let Some(delta) = report.effect.standby_cleanup_delta_bytes {
        output.push_str(&format!(
            "System cleanup delta: {}\n",
            format_signed_bytes(delta)
        ));
    }
    if let Some(reason) = &report.effect.system_cleanup_skipped_reason {
        output.push_str(&format!("System cleanup reason: {reason}\n"));
    }
    if report.available_increase().unsigned_abs() < 256 * 1024 * 1024 {
        output.push_str("Small-effect explanation:\n");
        output.push_str("1. Most background processes may already have low working sets.\n");
        output.push_str(
            "2. Foreground, system boundary, or threshold rules may have skipped processes.\n",
        );
        output.push_str("3. System-level standby list cleanup may be disabled.\n");
        output
            .push_str("4. Windows may keep pages in standby/cache state after process trimming.\n");
    }
    output.push('\n');

    output.push_str("Top handled processes:\n");
    let top = top_trimmed(report, 10);
    if top.is_empty() {
        output.push_str("(none)\n");
    } else {
        for result in top {
            output.push_str(&format!(
                "{:<24} {:>8} -> {:>8}\n",
                result.name,
                format_working_set(result.before_working_set),
                format_optional_working_set(result.after_working_set)
            ));
        }
    }
    output
}

fn format_dry_run(report: &OptimizeReport, verbose: bool) -> String {
    let mut output = String::new();
    output.push_str("Dry Run - 25% Target Preview\n\n");
    output.push_str("Will handle working sets:\n");
    let will_trim: Vec<&TrimResult> = report
        .results
        .iter()
        .filter(|result| matches!(result.status, TrimStatus::WouldTrim))
        .collect();
    if will_trim.is_empty() {
        output.push_str("(none)\n");
    } else {
        for result in will_trim {
            output.push_str(&format!(
                "{:<24} {:>8}    reason: background large working set\n",
                result.name,
                format_working_set(result.before_working_set)
            ));
        }
    }
    output.push_str("\nSkipped summary:\n");
    let skipped: Vec<&TrimResult> = report
        .results
        .iter()
        .filter(|result| matches!(result.status, TrimStatus::Skipped(_)))
        .collect();
    if skipped.is_empty() {
        output.push_str("(none)\n");
    } else {
        let counts = skip_summary_counts(report);
        for (label, count) in counts {
            output.push_str(&format!("{label:<24} {count:>5}\n"));
        }
    }

    if verbose {
        output.push_str("\nSkipped details:\n");
        for result in skipped {
            if let TrimStatus::Skipped(reason) = &result.status {
                output.push_str(&format!("{:<24} reason: {}\n", result.name, reason));
            }
        }
    } else {
        output.push_str("\nUse --verbose to show all skipped processes.\n");
    }
    output.push_str(&format!(
        "\nPlanned working-set rounds: {}\n",
        report.effect.working_set_rounds.max(1)
    ));
    if let Some(reason) = &report.effect.system_cleanup_skipped_reason {
        output.push_str(&format!("System cleanup: skipped ({reason})\n"));
    } else if report.effect.system_cleanup_executed {
        output.push_str("System cleanup: would execute when not dry-run\n");
    } else {
        output.push_str("System cleanup: skipped\n");
    }
    output
}

fn format_snapshot(snapshot: &MemorySnapshot) -> String {
    let used_percent = memory_load_percent(snapshot);
    format!(
        "Physical Used: {} / {} ({:.1}%)\nAvailable:     {}\nSystem Cache:  {}\nCommit:        {} / {}\nProcesses:     {}\n",
        format_gb(snapshot.used_physical()),
        format_gb(snapshot.total_physical),
        used_percent,
        format_gb(snapshot.available_physical),
        format_gb(snapshot.system_cache),
        format_gb(snapshot.commit_total),
        format_gb(snapshot.commit_limit),
        snapshot.process_count
    )
}

fn memory_load_percent(snapshot: &MemorySnapshot) -> f64 {
    let total = snapshot.total_physical.max(1);
    snapshot.used_physical() as f64 * 100.0 / total as f64
}

fn target_reached(report: &OptimizeReport) -> bool {
    !report.dry_run && memory_load_percent(&report.after) <= TARGET_MEMORY_LOAD_PERCENT
}

fn system_release_value_text(report: &OptimizeReport) -> &'static str {
    if report.dry_run {
        "preview"
    } else if report.effect.system_cleanup_executed {
        "released"
    } else if report.effect.system_cleanup_skipped_reason.is_some() {
        "checked"
    } else {
        "no visible change"
    }
}

fn top_trimmed(report: &OptimizeReport, limit: usize) -> Vec<&TrimResult> {
    let mut rows: Vec<&TrimResult> = report
        .results
        .iter()
        .filter(|result| matches!(result.status, TrimStatus::Trimmed))
        .collect();
    rows.sort_by_key(|result| {
        let reduction = result
            .after_working_set
            .map(|after| result.before_working_set.saturating_sub(after))
            .unwrap_or(0);
        std::cmp::Reverse(reduction)
    });
    rows.truncate(limit);
    rows
}

fn format_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
}

fn format_mb(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1024.0 / 1024.0)
}

pub fn format_working_set(bytes: u64) -> String {
    if bytes == 0 {
        "0 MB".to_owned()
    } else if bytes < 1024 * 1024 {
        "<1 MB".to_owned()
    } else {
        format_mb(bytes)
    }
}

pub fn format_optional_working_set(bytes: Option<u64>) -> String {
    match bytes {
        Some(bytes) => format_working_set(bytes),
        None => "unknown".to_owned(),
    }
}

fn format_signed_bytes(bytes: i64) -> String {
    let sign = if bytes >= 0 { "+" } else { "-" };
    let abs = bytes.unsigned_abs();
    format!("{}{}", sign, format_gb(abs))
}

#[allow(dead_code)]
fn reason_label(reason: &SkipReason) -> String {
    reason.to_string()
}

fn skip_summary_counts(report: &OptimizeReport) -> Vec<(&'static str, usize)> {
    let mut system = 0;
    let mut boundary = 0;
    let mut legacy_recommended = 0;
    let mut legacy_user = 0;
    let mut foreground = 0;
    let mut self_process = 0;
    let mut small = 0;
    let mut access_denied = 0;
    let mut open_failed = 0;
    let mut query_failed = 0;
    let mut exited = 0;
    let mut high_cpu = 0;
    let mut high_io = 0;
    let mut other = 0;

    for result in &report.results {
        if let TrimStatus::Skipped(reason) = &result.status {
            match reason {
                SkipReason::SystemProcess => system += 1,
                SkipReason::SystemBoundaryProcess => boundary += 1,
                SkipReason::LegacyRecommendedRule => legacy_recommended += 1,
                SkipReason::LegacyUserRule => legacy_user += 1,
                SkipReason::ForegroundProcess => foreground += 1,
                SkipReason::SelfProcess => self_process += 1,
                SkipReason::WorkingSetTooSmall => small += 1,
                SkipReason::AccessDenied => access_denied += 1,
                SkipReason::OpenProcessFailed => open_failed += 1,
                SkipReason::QueryFailed => query_failed += 1,
                SkipReason::ProcessExited => exited += 1,
                SkipReason::HighCpuUsage => high_cpu += 1,
                SkipReason::HighIoActivity => high_io += 1,
                SkipReason::Other(_) => other += 1,
            }
        }
    }

    [
        ("System process:", system),
        ("System boundary:", boundary),
        ("Legacy recommended rule:", legacy_recommended),
        ("Legacy user rule:", legacy_user),
        ("Working set too small:", small),
        ("Process query failed:", query_failed),
        ("Foreground process:", foreground),
        ("Self process:", self_process),
        ("Access denied:", access_denied),
        ("OpenProcess failed:", open_failed),
        ("Process exited:", exited),
        ("High CPU usage:", high_cpu),
        ("High I/O activity:", high_io),
        ("Other:", other),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn previous_report_is_archived_before_overwrite() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "memspark-report-history-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let report_path = root.join("last_report.json");
        let history_dir = root.join("history");
        let log_dir = root.join("logs");

        let mut config = AppConfig::default();
        config.report.last_report_path = report_path.to_string_lossy().to_string();
        config.report.history_report_dir = history_dir.to_string_lossy().to_string();
        config.report.developer_log_dir = log_dir.to_string_lossy().to_string();

        let first = sample_report(100);
        let second = sample_report(200);

        save_report(&config, &first)?;
        save_report(&config, &second)?;

        let history_path = history_dir.join("report-100.json");
        assert!(history_path.exists());
        assert_eq!(load_report_from_path(&history_path)?.timestamp_unix, 100);
        assert_eq!(load_last_report(&config)?.timestamp_unix, 200);

        assert_eq!(clear_report_history(&config)?, 1);
        assert!(!history_path.exists());

        let log_path = save_developer_log(&config, &second)?.expect("developer log path");
        let log_content = fs::read_to_string(&log_path)?;
        assert!(log_content.contains("MemSpark Developer Log"));
        assert!(log_content.contains("[Raw Report JSON]"));
        assert!(log_content.contains("\"timestamp_unix\": 200"));
        assert_eq!(clear_developer_logs(&config)?, 1);
        assert!(!log_path.exists());

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    fn sample_report(timestamp_unix: u64) -> OptimizeReport {
        OptimizeReport {
            mode: OptimizeMode::MemoryOptimization,
            dry_run: false,
            timestamp_unix,
            duration_ms: 10,
            before: sample_snapshot(2 * 1024 * 1024 * 1024),
            after: sample_snapshot(4 * 1024 * 1024 * 1024),
            scanned_count: 0,
            trimmed_count: 0,
            skipped_count: 0,
            failed_count: 0,
            effect: CleanupEffectSummary::default(),
            results: Vec::new(),
        }
    }

    fn sample_snapshot(available_physical: u64) -> MemorySnapshot {
        MemorySnapshot {
            total_physical: 8 * 1024 * 1024 * 1024,
            available_physical,
            memory_load_percent: 50,
            commit_total: 0,
            commit_limit: 0,
            system_cache: 0,
            process_count: 0,
            page_file_total: 0,
            page_file_available: 0,
            virtual_total: 0,
            virtual_available: 0,
        }
    }
}
