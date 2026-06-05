#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

slint::include_modules!();

mod i18n;

use crate::i18n::{detect_ui_language, texts_for_language, UiLanguage, UiTexts};
use memspark_core::config::{
    app_data_dir, default_developer_log_dir, default_report_history_dir, expand_env_path,
    init_config, load_config_or_default, save_config,
};
use memspark_core::{
    clear_developer_logs, clear_report_history, get_memory_snapshot, is_running_as_admin,
    load_last_report, optimize, run_elevated_and_wait, save_developer_log, save_report,
    MemorySnapshot, OptimizeReport, SkipReason, TrimResult, TrimStatus,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

const TARGET_MEMORY_LOAD_PERCENT: f64 = 25.0;

fn ui_is_english(texts: &UiTexts) -> bool {
    matches!(texts.locale, UiLanguage::EnUs)
}

fn main() -> Result<(), slint::PlatformError> {
    if let Some(exit_code) = run_elevated_child_if_requested() {
        std::process::exit(exit_code);
    }

    let app = AppWindow::new()?;
    let ui_settings = Arc::new(Mutex::new(load_ui_settings()));
    let texts = {
        let settings = current_ui_settings(&ui_settings);
        texts_for_language(settings.language)
    };
    let settings_draft = Arc::new(Mutex::new(load_settings_draft(&current_ui_settings(
        &ui_settings,
    ))));
    let callback_state = CallbackState {
        dashboard_active: Arc::new(AtomicBool::new(true)),
        optimize_busy: Arc::new(AtomicBool::new(false)),
        refresh_in_progress: Arc::new(AtomicBool::new(false)),
        ui_settings,
        settings_draft,
    };

    apply_ui_texts(&app, texts);
    install_callbacks(&app, texts, callback_state.clone());
    refresh_dashboard_async(
        app.as_weak(),
        Arc::clone(&callback_state.refresh_in_progress),
        texts,
    );
    load_memory_hardware_async(app.as_weak(), texts);
    start_dashboard_refresh_loop(
        app.as_weak(),
        Arc::clone(&callback_state.dashboard_active),
        Arc::clone(&callback_state.refresh_in_progress),
        texts,
    );
    refresh_report(&app, texts);
    refresh_settings(
        &app,
        texts,
        &callback_state.ui_settings,
        &callback_state.settings_draft,
    );

    app.run()
}

fn install_callbacks(app: &AppWindow, texts: &'static UiTexts, state: CallbackState) {
    let weak = app.as_weak();
    let refresh_flag = Arc::clone(&state.refresh_in_progress);
    app.on_refresh_status(move || {
        refresh_dashboard_async(weak.clone(), Arc::clone(&refresh_flag), texts);
    });

    let weak = app.as_weak();
    let active = Arc::clone(&state.dashboard_active);
    let refresh_flag = Arc::clone(&state.refresh_in_progress);
    let settings_state = Arc::clone(&state.ui_settings);
    let draft_state = Arc::clone(&state.settings_draft);
    app.on_page_changed(move |page| {
        let live_refresh_page = page.as_str() == "Dashboard";
        active.store(live_refresh_page, Ordering::Release);
        if let Some(app) = weak.upgrade() {
            if live_refresh_page {
                refresh_dashboard_async(app.as_weak(), Arc::clone(&refresh_flag), texts);
            } else if page.as_str() == "Settings" {
                refresh_settings(&app, texts, &settings_state, &draft_state);
            }
        }
    });

    let weak = app.as_weak();
    app.on_load_report(move || {
        if let Some(app) = weak.upgrade() {
            refresh_report(&app, texts);
        }
    });

    let weak = app.as_weak();
    let settings_state = Arc::clone(&state.ui_settings);
    let draft_state = Arc::clone(&state.settings_draft);
    app.on_init_config(move || {
        if let Some(app) = weak.upgrade() {
            match init_config(None, false) {
                Ok(path) => {
                    app.set_log_text(format!("{}{}", texts.config_ready, path.display()).into())
                }
                Err(err) => app.set_log_text(format!("{}{err}", texts.config_init_failed).into()),
            }
            refresh_settings(&app, texts, &settings_state, &draft_state);
        }
    });

    let weak = app.as_weak();
    let draft_state = Arc::clone(&state.settings_draft);
    app.on_select_language(move |language| {
        if let Some(app) = weak.upgrade() {
            if let Some(language) = parse_ui_language(language.as_str()) {
                let mut draft = current_settings_draft(&draft_state);
                draft.language = language;
                set_settings_draft(&draft_state, draft.clone());
                apply_settings_draft_to_ui(&app, &draft, texts);
                let message = if ui_is_english(texts) {
                    "Language changed. Click Save and Apply to restart."
                } else {
                    "语言已变更，点击“保存并应用设置”后重启生效。"
                };
                app.set_log_text(message.into());
            }
        }
    });

    let weak = app.as_weak();
    let draft_state = Arc::clone(&state.settings_draft);
    app.on_select_theme(move |theme| {
        if let Some(app) = weak.upgrade() {
            let Some(theme) = parse_ui_theme(theme.as_str()) else {
                return;
            };
            let mut draft = current_settings_draft(&draft_state);
            draft.theme = theme.to_owned();
            set_settings_draft(&draft_state, draft.clone());
            apply_settings_draft_to_ui(&app, &draft, texts);
            let message = if ui_is_english(texts) {
                "Theme changed. Click Save and Apply to keep it."
            } else {
                "主题已变更，点击“保存并应用设置”后保留。"
            };
            app.set_log_text(message.into());
        }
    });

    let weak = app.as_weak();
    let draft_state = Arc::clone(&state.settings_draft);
    app.on_open_report(move || {
        if let Some(app) = weak.upgrade() {
            open_report_path(
                &app,
                current_settings_draft(&draft_state).report_path,
                texts,
            );
        }
    });

    let weak = app.as_weak();
    let draft_state = Arc::clone(&state.settings_draft);
    app.on_open_report_folder(move || {
        if let Some(app) = weak.upgrade() {
            open_report_folder_path(
                &app,
                current_settings_draft(&draft_state).report_path,
                texts,
            );
        }
    });

    let weak = app.as_weak();
    let draft_state = Arc::clone(&state.settings_draft);
    app.on_choose_report_path(move || {
        choose_report_folder_async(weak.clone(), Arc::clone(&draft_state), texts);
    });

    let weak = app.as_weak();
    app.on_clear_report_history(move || {
        if let Some(app) = weak.upgrade() {
            match load_config_or_default(None).and_then(|config| clear_report_history(&config)) {
                Ok(removed) if ui_is_english(texts) => {
                    app.set_log_text(format!("Cleared history reports: {removed} file(s).").into())
                }
                Ok(removed) => {
                    app.set_log_text(format!("已清理历史报告：{removed} 个文件。").into())
                }
                Err(err) if ui_is_english(texts) => {
                    app.set_log_text(format!("Failed to clear history reports: {err}").into())
                }
                Err(err) => app.set_log_text(format!("清理历史报告失败：{err}").into()),
            }
        }
    });

    let weak = app.as_weak();
    app.on_clear_developer_logs(move || {
        if let Some(app) = weak.upgrade() {
            match load_config_or_default(None).and_then(|config| clear_developer_logs(&config)) {
                Ok(removed) if ui_is_english(texts) => {
                    app.set_log_text(format!("Cleared developer logs: {removed} file(s).").into())
                }
                Ok(removed) => {
                    app.set_log_text(format!("已清理开发日志：{removed} 个文件。").into())
                }
                Err(err) if ui_is_english(texts) => {
                    app.set_log_text(format!("Failed to clear developer logs: {err}").into())
                }
                Err(err) => app.set_log_text(format!("清理开发日志失败：{err}").into()),
            }
        }
    });

    let weak = app.as_weak();
    let settings_state = Arc::clone(&state.ui_settings);
    let draft_state = Arc::clone(&state.settings_draft);
    app.on_apply_settings(move || {
        if let Some(app) = weak.upgrade() {
            save_settings_and_restart(&app, &settings_state, &draft_state, texts);
        }
    });

    let weak = app.as_weak();
    let busy_flag = Arc::clone(&state.optimize_busy);
    let refresh_flag = Arc::clone(&state.refresh_in_progress);
    app.on_run_trim(move |mode, dry_run| {
        let mode_text = mode.to_string();
        if let Some(app) = weak.upgrade() {
            if busy_flag.swap(true, Ordering::AcqRel) {
                app.set_task_feedback_visible(true);
                app.set_task_status_text(texts.optimize_running.into());
                app.set_task_progress_ratio(0.35);
                app.set_log_text(texts.optimize_running.into());
                return;
            }
            app.set_busy(true);
            app.set_task_feedback_visible(true);
            app.set_task_status_text(texts.optimize_running.into());
            app.set_task_progress_ratio(0.08);
            app.set_log_text(texts.optimization_started.into());
        }

        let weak_for_thread = weak.clone();
        let busy_for_thread = Arc::clone(&busy_flag);
        let refresh_for_thread = Arc::clone(&refresh_flag);
        std::thread::spawn(move || {
            let started = Instant::now();
            let progress_weak = weak_for_thread.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = progress_weak.upgrade() {
                    app.set_task_status_text(texts.optimize_running.into());
                    app.set_task_progress_ratio(0.55);
                }
            });
            let result = run_optimize_task(&mode_text, dry_run, texts);
            keep_feedback_visible(started);
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = weak_for_thread.upgrade() {
                    app.set_busy(false);
                    busy_for_thread.store(false, Ordering::Release);
                    match result {
                        Ok(data) => {
                            apply_report_view_data(&app, data);
                            app.set_task_status_text(texts.optimization_completed.into());
                            app.set_task_progress_ratio(1.0);
                            app.set_log_text(texts.optimization_completed.into());
                            refresh_dashboard_async(
                                app.as_weak(),
                                Arc::clone(&refresh_for_thread),
                                texts,
                            );
                        }
                        Err(err) => {
                            let message = format!("{}{err}", texts.optimization_failed);
                            app.set_task_status_text(message.clone().into());
                            app.set_task_progress_ratio(1.0);
                            app.set_log_text(message.into())
                        }
                    }
                }
            });
        });
    });
}

fn apply_ui_texts(app: &AppWindow, texts: &UiTexts) {
    app.set_task_feedback_visible(false);
    app.set_task_status_text(texts.ready.into());
    app.set_task_progress_ratio(0.0);
    app.set_text_app_subtitle(texts.app_subtitle.into());
    app.set_text_nav_dashboard(texts.nav_dashboard.into());
    app.set_text_nav_report(texts.nav_report.into());
    app.set_text_nav_settings(texts.nav_settings.into());
    app.set_text_nav_about(texts.nav_about.into());
    app.set_text_dashboard_title(texts.dashboard_title.into());
    app.set_text_dashboard_tagline(texts.dashboard_tagline.into());
    app.set_text_memory_hardware(texts.memory_hardware.into());
    app.set_text_current_memory(texts.current_memory.into());
    app.set_text_used_memory(texts.used_memory.into());
    app.set_text_memory_overview(texts.memory_overview.into());
    app.set_text_available_memory(texts.available_memory.into());
    app.set_text_used_ratio(texts.used_ratio.into());
    app.set_text_available_ratio(texts.available_ratio.into());
    app.set_text_commit_usage(texts.commit_usage.into());
    app.set_text_system_cache(texts.system_cache.into());
    app.set_text_process_count(texts.process_count.into());
    app.set_text_no_kill(texts.no_kill.into());
    app.set_text_no_kill_desc(texts.no_kill_desc.into());
    app.set_text_no_inject(texts.no_inject.into());
    app.set_text_no_inject_desc(texts.no_inject_desc.into());
    app.set_text_manual_only(texts.manual_only.into());
    app.set_text_manual_only_desc(texts.manual_only_desc.into());
    app.set_text_no_default_cache_cleanup(texts.no_default_cache_cleanup.into());
    app.set_text_no_default_cache_cleanup_desc(texts.no_default_cache_cleanup_desc.into());
    app.set_text_normal_optimize(texts.normal_optimize.into());
    app.set_text_strong_optimize(texts.strong_optimize.into());
    app.set_text_refresh_status(texts.refresh_status.into());
    app.set_text_refreshing(texts.refreshing.into());
    app.set_text_auto_refresh(texts.auto_refresh.into());
    app.set_text_available_hint(texts.available_hint.into());
    app.set_text_commit_hint(texts.commit_hint.into());
    app.set_text_cache_hint(texts.cache_hint.into());
    app.set_text_process_hint(texts.process_hint.into());
    app.set_text_report_conclusion_section(texts.report_conclusion_section.into());
    app.set_text_report_core_results(texts.report_core_results.into());
    app.set_text_report_target_hint(texts.report_target_hint.into());
    app.set_text_report_working_set_hint(texts.report_working_set_hint.into());
    app.set_text_report_before_after(texts.report_before_after.into());
    app.set_text_report_change_note(texts.report_change_note.into());
    app.set_text_report_process_table_header(texts.report_process_table_header.into());
    app.set_text_report_skip_summary_section(texts.report_skip_summary_section.into());
    app.set_text_report_skip_table_header(texts.report_skip_table_header.into());
    app.set_text_report_title(texts.report_title.into());
    app.set_text_reload_last_report(texts.reload_last_report.into());
    app.set_text_before(texts.before.into());
    app.set_text_after(texts.after.into());
    app.set_text_available_increased(texts.available_increased.into());
    app.set_text_processes_scanned(texts.processes_scanned.into());
    app.set_text_processes_trimmed(texts.processes_trimmed.into());
    app.set_text_processes_skipped(texts.processes_skipped.into());
    app.set_text_failed(texts.failed.into());
    app.set_text_top_trimmed_processes(texts.top_trimmed_processes.into());
    app.set_text_detailed_report(texts.detailed_report.into());
    app.set_text_settings_title(texts.settings_title.into());
    app.set_text_settings_hint(texts.settings_hint.into());
    app.set_text_reports(texts.reports.into());
    app.set_text_initialize_config(texts.initialize_config.into());
    app.set_text_language(texts.language.into());
    app.set_text_simplified_chinese(texts.simplified_chinese.into());
    app.set_text_english(texts.english.into());
    app.set_text_language_restart_hint(texts.language_restart_hint.into());
    app.set_text_theme(texts.theme.into());
    app.set_text_dark(texts.dark.into());
    app.set_text_light(texts.light.into());
    app.set_text_reports_desc(texts.reports_desc.into());
    app.set_text_settings_apply(texts.settings_apply.into());
    app.set_text_theme_dark_ready(texts.theme_dark_ready.into());
    app.set_text_settings_open_report(texts.settings_open_report.into());
    app.set_text_settings_open_folder(texts.settings_open_folder.into());
    app.set_text_settings_choose_location(texts.settings_choose_location.into());
    app.set_text_settings_clear_history(texts.settings_clear_history.into());
    app.set_text_settings_clear_logs(texts.settings_clear_logs.into());
    app.set_text_settings_choose_save_location(texts.settings_choose_save_location.into());
    app.set_text_settings_clear_report_history(texts.settings_clear_report_history.into());
    app.set_text_settings_clear_developer_logs(texts.settings_clear_developer_logs.into());
    app.set_text_about_title(texts.about_title.into());
    app.set_text_about_description(texts.about_description.into());
    app.set_text_safety_boundary(texts.safety_boundary.into());
    app.set_text_no_process_killing(texts.no_process_killing.into());
    app.set_text_no_process_injection(texts.no_process_injection.into());
    app.set_text_no_memory_modification(texts.no_memory_modification.into());
    app.set_text_manual_execution_only(texts.manual_execution_only.into());
    app.set_text_about_system_boundary_desc(texts.about_system_boundary_desc.into());
    app.set_text_about_no_injection_desc(texts.about_no_injection_desc.into());
    app.set_text_about_no_memory_modification_desc(texts.about_no_memory_modification_desc.into());
    app.set_text_about_manual_execution_desc(texts.about_manual_execution_desc.into());
    app.set_text_license(texts.license.into());
    app.set_text_github(texts.github.into());
    app.set_report_text(texts.no_report_loaded.into());
    apply_report_view_data(app, ReportViewData::empty(texts));
    app.set_log_text(texts.ready.into());
    app.set_memory_hardware_summary(texts.loading.into());
}

fn start_dashboard_refresh_loop(
    weak: slint::Weak<AppWindow>,
    dashboard_active: Arc<AtomicBool>,
    refresh_in_progress: Arc<AtomicBool>,
    texts: &'static UiTexts,
) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(2));
        if !dashboard_active.load(Ordering::Acquire) {
            continue;
        }
        refresh_dashboard_async(weak.clone(), Arc::clone(&refresh_in_progress), texts);
    });
}

#[derive(Debug)]
struct DashboardSnapshot {
    memory_usage_percent: f32,
    memory_usage_ratio: f32,
    available_ratio: f32,
    commit_ratio: f32,
    cache_ratio: f32,
    process_ratio: f32,
    memory_usage_text: String,
    physical_used_text: String,
    physical_total_text: String,
    available_text: String,
    available_percent_text: String,
    commit_text: String,
    system_cache_text: String,
    process_count_text: String,
}

#[derive(Debug, Clone)]
struct MemoryHardwareInfo {
    total_capacity_bytes: u64,
    module_count: usize,
    slot_count: Option<usize>,
    memory_type: Option<String>,
    speed_mt_s: Option<u32>,
    modules: Vec<MemoryModuleInfo>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct MemoryModuleInfo {
    capacity_bytes: u64,
    speed_mt_s: Option<u32>,
    manufacturer: Option<String>,
    part_number: Option<String>,
    device_locator: Option<String>,
    memory_type: Option<String>,
}

#[derive(Clone)]
struct CallbackState {
    dashboard_active: Arc<AtomicBool>,
    optimize_busy: Arc<AtomicBool>,
    refresh_in_progress: Arc<AtomicBool>,
    ui_settings: Arc<Mutex<UiSettings>>,
    settings_draft: Arc<Mutex<SettingsDraft>>,
}

#[derive(Debug, Clone)]
struct UiSettings {
    language: UiLanguage,
    theme: String,
}

#[derive(Debug, Clone)]
struct SettingsDraft {
    language: UiLanguage,
    theme: String,
    report_path: PathBuf,
    report_history_dir: PathBuf,
    developer_log_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct ReportViewData {
    text: String,
    before_memory: String,
    after_memory: String,
    available_increase: String,
    scanned: String,
    trimmed: String,
    skipped: String,
    failed: String,
    top_trimmed: String,
    primary_process_title: String,
    conclusion: String,
    report_type: String,
    mode: String,
    duration: String,
    available_change_detail: String,
    system_release_detail: String,
    skip_summary: String,
}

impl ReportViewData {
    fn empty(texts: &UiTexts) -> Self {
        Self {
            text: texts.no_report_loaded.to_owned(),
            before_memory: "--".to_owned(),
            after_memory: "--".to_owned(),
            available_increase: "--".to_owned(),
            scanned: "--".to_owned(),
            trimmed: "--".to_owned(),
            skipped: "--".to_owned(),
            failed: "--".to_owned(),
            top_trimmed: texts.none.to_owned(),
            primary_process_title: texts.top_trimmed_processes.to_owned(),
            conclusion: texts.no_report_loaded.to_owned(),
            report_type: "--".to_owned(),
            mode: "--".to_owned(),
            duration: "--".to_owned(),
            available_change_detail: texts.no_report_loaded.to_owned(),
            system_release_detail: texts.no_report_loaded.to_owned(),
            skip_summary: texts.none.to_owned(),
        }
    }
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            language: detect_ui_language(),
            theme: "dark".to_owned(),
        }
    }
}

fn refresh_dashboard_async(
    weak: slint::Weak<AppWindow>,
    refresh_in_progress: Arc<AtomicBool>,
    texts: &'static UiTexts,
) {
    if refresh_in_progress.swap(true, Ordering::AcqRel) {
        return;
    }

    let weak_loading = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = weak_loading.upgrade() {
            app.set_is_refreshing(true);
        }
    });

    std::thread::spawn(move || {
        let snapshot = read_dashboard_snapshot();
        let done = Arc::clone(&refresh_in_progress);
        let result = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                apply_dashboard_snapshot(&app, snapshot, texts);
            }
            done.store(false, Ordering::Release);
        });
        if result.is_err() {
            refresh_in_progress.store(false, Ordering::Release);
        }
    });
}

fn read_dashboard_snapshot() -> Result<DashboardSnapshot, String> {
    let snapshot = get_memory_snapshot().map_err(|err| err.to_string())?;
    let total = snapshot.total_physical.max(1);
    let used = snapshot.used_physical();
    let percent = used as f64 * 100.0 / total as f64;
    let percent = percent.clamp(0.0, 100.0);
    let commit_ratio = ratio(snapshot.commit_total, snapshot.commit_limit);
    let available_ratio = ratio(snapshot.available_physical, snapshot.total_physical);
    let cache_ratio = ratio(snapshot.system_cache, snapshot.total_physical);
    let process_ratio = (snapshot.process_count as f64 / 500.0).clamp(0.0, 1.0) as f32;

    Ok(DashboardSnapshot {
        memory_usage_percent: percent as f32,
        memory_usage_ratio: (percent / 100.0) as f32,
        available_ratio,
        commit_ratio,
        cache_ratio,
        process_ratio,
        memory_usage_text: format!("{percent:.1}%"),
        physical_used_text: gb(used),
        physical_total_text: gb(snapshot.total_physical),
        available_text: gb(snapshot.available_physical),
        available_percent_text: format!("{:.1}%", 100.0 - percent),
        commit_text: format!(
            "{} / {}",
            gb(snapshot.commit_total),
            gb(snapshot.commit_limit)
        ),
        system_cache_text: gb(snapshot.system_cache),
        process_count_text: snapshot.process_count.to_string(),
    })
}

fn load_memory_hardware_async(weak: slint::Weak<AppWindow>, texts: &'static UiTexts) {
    std::thread::spawn(move || {
        let summary = match read_memory_hardware_info() {
            Ok(info) => format_memory_hardware_summary(&info, texts),
            Err(_) => texts.hardware_unavailable.to_owned(),
        };
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                app.set_memory_hardware_summary(summary.into());
            }
        });
    });
}

#[cfg(windows)]
fn read_memory_hardware_info() -> Result<MemoryHardwareInfo, String> {
    let script = r#"
$ErrorActionPreference = 'Stop'
Get-CimInstance Win32_PhysicalMemory | ForEach-Object {
    $manufacturer = ([string]$_.Manufacturer) -replace '\|','/'
    $part = ([string]$_.PartNumber) -replace '\|','/'
    $locator = ([string]$_.DeviceLocator) -replace '\|','/'
    'M|{0}|{1}|{2}|{3}|{4}|{5}|{6}|{7}' -f $_.Capacity,$_.SMBIOSMemoryType,$_.MemoryType,$_.Speed,$_.ConfiguredClockSpeed,$manufacturer,$part,$locator
}
Get-CimInstance Win32_PhysicalMemoryArray | ForEach-Object {
    'A|{0}' -f $_.MemoryDevices
}
"#;

    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .map_err(|err| err.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }

    parse_memory_hardware_output(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(windows))]
fn read_memory_hardware_info() -> Result<MemoryHardwareInfo, String> {
    Err("memory hardware information is only available on Windows".to_owned())
}

fn parse_memory_hardware_output(output: &str) -> Result<MemoryHardwareInfo, String> {
    let mut modules = Vec::new();
    let mut slot_count: Option<usize> = None;

    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Some(rest) = line.strip_prefix("M|") {
            let fields: Vec<&str> = rest.split('|').collect();
            if fields.len() < 8 {
                continue;
            }

            let capacity_bytes = parse_u64_field(fields[0]).unwrap_or(0);
            if capacity_bytes == 0 {
                continue;
            }

            let smbios_type = parse_u32_field(fields[1]);
            let legacy_type = parse_u32_field(fields[2]);
            let speed = parse_u32_field(fields[4]).or_else(|| parse_u32_field(fields[3]));

            modules.push(MemoryModuleInfo {
                capacity_bytes,
                speed_mt_s: speed,
                manufacturer: non_empty_field(fields[5]),
                part_number: non_empty_field(fields[6]),
                device_locator: non_empty_field(fields[7]),
                memory_type: memory_type_label(smbios_type, legacy_type).map(str::to_owned),
            });
        } else if let Some(rest) = line.strip_prefix("A|") {
            if let Some(count) = parse_u32_field(rest).filter(|count| *count > 0) {
                slot_count = Some(slot_count.map_or(count as usize, |old| old.max(count as usize)));
            }
        }
    }

    if modules.is_empty() {
        return Err("no physical memory modules reported".to_owned());
    }

    let total_capacity_bytes = modules.iter().map(|module| module.capacity_bytes).sum();
    let module_count = modules.len();
    let memory_type = common_memory_type(&modules);
    let speed_mt_s = common_speed(&modules);

    Ok(MemoryHardwareInfo {
        total_capacity_bytes,
        module_count,
        slot_count,
        memory_type,
        speed_mt_s,
        modules,
    })
}

fn format_memory_hardware_summary(info: &MemoryHardwareInfo, texts: &UiTexts) -> String {
    if info.module_count == 0 || info.total_capacity_bytes == 0 {
        return texts.hardware_unavailable.to_owned();
    }

    let memory_type = info
        .memory_type
        .clone()
        .unwrap_or_else(|| texts.unknown.to_owned());
    let total = format_capacity_gb(info.total_capacity_bytes);
    let module_layout = format_module_layout(&info.modules, texts);
    let speed = format_speed_summary(info, texts);
    let slots = match info.slot_count {
        Some(total_slots) => format!(
            "{} {} / {}",
            texts.slots_label, info.module_count, total_slots
        ),
        None => format!(
            "{} {} / {}",
            texts.slots_label, info.module_count, texts.unknown
        ),
    };

    format!("{memory_type} · {total} · {module_layout} · {speed} · {slots}")
}

fn format_module_layout(modules: &[MemoryModuleInfo], texts: &UiTexts) -> String {
    let capacities: Vec<u64> = modules
        .iter()
        .map(|module| module.capacity_bytes)
        .filter(|capacity| *capacity > 0)
        .collect();

    if capacities.is_empty() {
        return texts.unknown.to_owned();
    }

    if capacities.iter().all(|capacity| *capacity == capacities[0]) {
        format!(
            "{} × {}",
            capacities.len(),
            format_capacity_gb(capacities[0])
        )
    } else {
        capacities
            .iter()
            .map(|capacity| format_capacity_gb(*capacity))
            .collect::<Vec<_>>()
            .join(" + ")
    }
}

fn format_speed_summary(info: &MemoryHardwareInfo, texts: &UiTexts) -> String {
    if let Some(speed) = info.speed_mt_s {
        return format!("{speed} MT/s");
    }

    let mut speeds: Vec<u32> = info
        .modules
        .iter()
        .filter_map(|module| module.speed_mt_s)
        .filter(|speed| *speed > 0)
        .collect();
    speeds.sort_unstable();
    speeds.dedup();

    match speeds.as_slice() {
        [] => texts.unknown_frequency.to_owned(),
        [speed] => format!("{speed} MT/s"),
        _ => {
            speeds
                .iter()
                .map(|speed| format!("{speed}"))
                .collect::<Vec<_>>()
                .join("/")
                + " MT/s"
        }
    }
}

fn common_memory_type(modules: &[MemoryModuleInfo]) -> Option<String> {
    let mut types: Vec<&str> = modules
        .iter()
        .filter_map(|module| module.memory_type.as_deref())
        .filter(|value| !value.is_empty())
        .collect();
    types.sort_unstable();
    types.dedup();

    match types.as_slice() {
        [] => None,
        [memory_type] => Some((*memory_type).to_owned()),
        _ => Some(types.join("/")),
    }
}

fn common_speed(modules: &[MemoryModuleInfo]) -> Option<u32> {
    let mut speeds: Vec<u32> = modules
        .iter()
        .filter_map(|module| module.speed_mt_s)
        .filter(|speed| *speed > 0)
        .collect();
    speeds.sort_unstable();
    speeds.dedup();
    if speeds.len() == 1 {
        speeds.first().copied()
    } else {
        None
    }
}

fn memory_type_label(smbios_type: Option<u32>, legacy_type: Option<u32>) -> Option<&'static str> {
    [smbios_type, legacy_type]
        .into_iter()
        .flatten()
        .find_map(|code| match code {
            20 => Some("DDR"),
            21 => Some("DDR2"),
            24 => Some("DDR3"),
            26 => Some("DDR4"),
            27 => Some("LPDDR"),
            28 => Some("LPDDR2"),
            29 => Some("LPDDR3"),
            30 => Some("LPDDR4"),
            34 => Some("DDR5"),
            35 => Some("LPDDR5"),
            _ => None,
        })
}

fn parse_u64_field(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok().filter(|value| *value > 0)
}

fn parse_u32_field(value: &str) -> Option<u32> {
    value.trim().parse::<u32>().ok().filter(|value| *value > 0)
}

fn non_empty_field(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

fn apply_dashboard_snapshot(
    app: &AppWindow,
    snapshot: Result<DashboardSnapshot, String>,
    texts: &UiTexts,
) {
    match snapshot {
        Ok(snapshot) => {
            app.set_memory_usage_percent(snapshot.memory_usage_percent);
            app.set_memory_usage_ratio(snapshot.memory_usage_ratio);
            app.set_available_ratio(snapshot.available_ratio);
            app.set_commit_ratio(snapshot.commit_ratio);
            app.set_cache_ratio(snapshot.cache_ratio);
            app.set_process_ratio(snapshot.process_ratio);
            app.set_memory_usage_text(snapshot.memory_usage_text.into());
            app.set_physical_used_text(snapshot.physical_used_text.into());
            app.set_physical_total_text(snapshot.physical_total_text.into());
            app.set_available_text(snapshot.available_text.into());
            app.set_available_percent_text(snapshot.available_percent_text.into());
            app.set_commit_text(snapshot.commit_text.into());
            app.set_system_cache_text(snapshot.system_cache_text.into());
            app.set_process_count_text(snapshot.process_count_text.into());
        }
        Err(err) => app.set_log_text(format!("{}{err}", texts.dashboard_refresh_failed).into()),
    }
    app.set_is_refreshing(false);
}

fn refresh_report(app: &AppWindow, texts: &UiTexts) {
    match load_config_or_default(None).and_then(|config| load_last_report(&config)) {
        Ok(report) => apply_report_view_data(app, report_view_data(&report, texts)),
        Err(err) => {
            let mut data = ReportViewData::empty(texts);
            data.text = format!("{}{err}", texts.no_report_available);
            apply_report_view_data(app, data);
        }
    }
}

fn apply_report_view_data(app: &AppWindow, data: ReportViewData) {
    app.set_report_text(data.text.into());
    app.set_report_before_memory_text(data.before_memory.into());
    app.set_report_after_memory_text(data.after_memory.into());
    app.set_report_available_increase_text(data.available_increase.into());
    app.set_report_scanned_text(data.scanned.into());
    app.set_report_trimmed_text(data.trimmed.into());
    app.set_report_skipped_text(data.skipped.into());
    app.set_report_failed_text(data.failed.into());
    app.set_report_top_trimmed_text(data.top_trimmed.into());
    app.set_report_primary_process_title(data.primary_process_title.into());
    app.set_report_conclusion_text(data.conclusion.into());
    app.set_report_type_text(data.report_type.into());
    app.set_report_mode_text(data.mode.into());
    app.set_report_duration_text(data.duration.into());
    app.set_report_available_change_detail_text(data.available_change_detail.into());
    app.set_report_system_release_detail_text(data.system_release_detail.into());
    app.set_report_skip_summary_text(data.skip_summary.into());
}

fn run_optimize_task(mode: &str, dry_run: bool, texts: &UiTexts) -> Result<ReportViewData, String> {
    let _mode = mode;
    let config = load_config_or_default(None).map_err(|err| err.to_string())?;
    let report = if !dry_run && !is_running_as_admin() {
        run_elevated_self_optimize()?
    } else {
        optimize(dry_run, &config).map_err(|err| err.to_string())?
    };
    let mut data = report_view_data(&report, texts);
    if let Err(err) = save_report(&config, &report) {
        data.text
            .push_str(&format!("\n{}{err}\n", texts.report_save_warning));
    }
    if let Err(err) = save_developer_log(&config, &report) {
        let warning = if ui_is_english(texts) {
            format!("\nwarning: developer log could not be saved: {err}\n")
        } else {
            format!("\n警告：开发日志无法保存：{err}\n")
        };
        data.text.push_str(&warning);
    }
    Ok(data)
}

fn run_elevated_self_optimize() -> Result<OptimizeReport, String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let output = elevated_output_path("ui");
    let error_output = output.with_extension("error.txt");
    let args = vec![
        "--elevated-child".to_owned(),
        "--elevated-output".to_owned(),
        output.display().to_string(),
    ];
    let exit_code = run_elevated_and_wait(&exe.display().to_string(), &join_windows_args(&args))
        .map_err(|err| err.to_string())?;
    if exit_code != 0 {
        let detail = fs::read_to_string(&error_output).ok();
        let _ = fs::remove_file(&error_output);
        return Err(match detail {
            Some(detail) if !detail.trim().is_empty() => {
                format!("管理员子进程退出码：{exit_code}；{detail}")
            }
            _ => format!("管理员子进程退出码：{exit_code}"),
        });
    }
    let content = fs::read_to_string(&output).map_err(|err| err.to_string())?;
    let parsed = serde_json::from_str(&content).map_err(|err| err.to_string())?;
    let _ = fs::remove_file(output);
    Ok(parsed)
}

fn run_elevated_child_if_requested() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    let mut is_child = false;
    let mut output_path = None;

    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--elevated-child" => is_child = true,
            "--elevated-output" => {
                output_path = args.next().map(PathBuf::from);
            }
            _ => {}
        }
    }

    if !is_child {
        return None;
    }

    let Some(output_path) = output_path else {
        return Some(2);
    };
    let error_output = output_path.with_extension("error.txt");

    match write_elevated_child_report(&output_path) {
        Ok(()) => Some(0),
        Err(err) => {
            let _ = fs::write(error_output, err);
            Some(1)
        }
    }
}

fn write_elevated_child_report(output_path: &PathBuf) -> Result<(), String> {
    let config = load_config_or_default(None).map_err(|err| err.to_string())?;
    let report = optimize(false, &config).map_err(|err| err.to_string())?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    fs::write(
        output_path,
        serde_json::to_string_pretty(&report).map_err(|err| err.to_string())?,
    )
    .map_err(|err| err.to_string())
}

fn elevated_output_path(kind: &str) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    std::env::temp_dir().join(format!("memspark-{kind}-{}-{now}.json", std::process::id()))
}

fn join_windows_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_windows_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_windows_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_owned();
    }
    if !arg.chars().any(|ch| ch.is_whitespace() || ch == '"') {
        return arg.to_owned();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                quoted.push(ch);
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

fn keep_feedback_visible(started: Instant) {
    let minimum = Duration::from_millis(450);
    let elapsed = started.elapsed();
    if elapsed < minimum {
        std::thread::sleep(minimum - elapsed);
    }
}

fn format_ui_report_text(report: &OptimizeReport, texts: &UiTexts) -> String {
    if report.dry_run {
        return format_ui_dry_run(report, texts);
    }

    let mut output = String::new();
    output.push_str(texts.memspark_report);
    output.push('\n');
    output.push_str(&format!(
        "{}: {}\n",
        texts.mode,
        target_status_label(report, texts)
    ));
    output.push_str(&format!(
        "{}: {:.2}s\n\n",
        texts.duration,
        report.duration_ms as f64 / 1000.0
    ));
    output.push_str(texts.before);
    output.push_str(":\n");
    output.push_str(&format_snapshot_text(&report.before, texts));
    output.push('\n');
    output.push_str(texts.after);
    output.push_str(":\n");
    output.push_str(&format_snapshot_text(&report.after, texts));
    output.push('\n');
    output.push_str(texts.result);
    output.push_str(":\n");
    output.push_str(&format!(
        "{}: {}\n",
        texts.processes_trimmed,
        final_memory_load_text(report)
    ));
    output.push_str(&format!(
        "{}: {}\n",
        texts.available_increased,
        format_signed_bytes(report.available_increase())
    ));
    output.push_str(&format!(
        "{}: {}\n",
        texts.processes_scanned, report.scanned_count
    ));
    output.push_str(&format!(
        "{}: {}\n",
        texts.processes_skipped,
        working_set_result_value(report)
    ));
    if ui_is_english(texts) {
        output.push_str(&format!(
            "Kept/skipped processes: {}\n",
            report.skipped_count
        ));
        output.push_str(&format!("Failed: {}\n", report.failed_count));
    } else {
        output.push_str(&format!("保留/跳过进程: {}\n", report.skipped_count));
        output.push_str(&format!("失败: {}\n", report.failed_count));
    }
    output.push_str(&format!(
        "{}: {}\n",
        texts.failed,
        system_release_value_text(report, texts)
    ));
    let system_release_detail_label = if ui_is_english(texts) {
        "System release detail"
    } else {
        "系统释放说明"
    };
    output.push_str(&format!(
        "{system_release_detail_label}: {}\n\n",
        system_release_detail_text(report, texts)
    ));

    output.push_str(texts.top_trimmed_processes);
    output.push_str(":\n");
    let top = top_trimmed(report, 10);
    if top.is_empty() {
        output.push_str(texts.none);
        output.push('\n');
    } else {
        for result in top {
            output.push_str(&format!(
                "{}    {} -> {}\n",
                result.name,
                format_working_set_ui(result.before_working_set),
                format_optional_working_set_ui(result.after_working_set, texts)
            ));
        }
    }
    output
}

fn report_view_data(report: &OptimizeReport, texts: &UiTexts) -> ReportViewData {
    ReportViewData {
        text: format_ui_report_text(report, texts),
        before_memory: memory_summary_text(&report.before, texts),
        after_memory: memory_summary_text(&report.after, texts),
        available_increase: format_signed_bytes(report.available_increase()),
        scanned: report.scanned_count.to_string(),
        trimmed: final_memory_load_text(report),
        skipped: working_set_result_value(report),
        failed: system_release_value_text(report, texts),
        top_trimmed: top_activity_text(report, texts),
        primary_process_title: if report.dry_run {
            texts.will_trim.to_owned()
        } else {
            texts.top_trimmed_processes.to_owned()
        },
        conclusion: report_conclusion_text(report, texts),
        report_type: if report.dry_run {
            texts.dry_run.to_owned()
        } else {
            texts.result.to_owned()
        },
        mode: target_status_label(report, texts),
        duration: format!("{:.2}s", report.duration_ms as f64 / 1000.0),
        available_change_detail: available_change_detail_text(report, texts),
        system_release_detail: system_release_detail_text(report, texts),
        skip_summary: skip_summary_text(report, texts),
    }
}

fn memory_summary_text(snapshot: &MemorySnapshot, texts: &UiTexts) -> String {
    let used_percent = memory_load_percent(snapshot);
    if ui_is_english(texts) {
        return format!(
            "Memory load: {:.1}%\nUsed memory: {}\nAvailable memory: {}\nTotal physical: {}",
            used_percent,
            gb(snapshot.used_physical()),
            gb(snapshot.available_physical),
            gb(snapshot.total_physical)
        );
    }
    format!(
        "内存占用：{:.1}%\n已用内存：{}\n可用内存：{}\n总物理内存：{}",
        used_percent,
        gb(snapshot.used_physical()),
        gb(snapshot.available_physical),
        gb(snapshot.total_physical)
    )
}

fn memory_load_percent(snapshot: &MemorySnapshot) -> f64 {
    let total = snapshot.total_physical.max(1);
    snapshot.used_physical() as f64 * 100.0 / total as f64
}

fn target_reached(report: &OptimizeReport) -> bool {
    !report.dry_run && memory_load_percent(&report.after) <= TARGET_MEMORY_LOAD_PERCENT
}

fn final_memory_load_text(report: &OptimizeReport) -> String {
    format!("{:.1}%", memory_load_percent(&report.after))
}

fn target_status_label(report: &OptimizeReport, texts: &UiTexts) -> String {
    match (ui_is_english(texts), report.dry_run, target_reached(report)) {
        (true, true, _) => "25% target preview".to_owned(),
        (true, false, true) => "25% target reached".to_owned(),
        (true, false, false) => "25% target not reached".to_owned(),
        (false, true, _) => "25% 目标预览".to_owned(),
        (false, false, true) => "25% 目标已达成".to_owned(),
        (false, false, false) => "25% 目标未达成".to_owned(),
    }
}

fn working_set_result_value(report: &OptimizeReport) -> String {
    format!("{} / {}", report.trimmed_count, report.scanned_count)
}

fn working_set_result_detail(report: &OptimizeReport, texts: &UiTexts) -> String {
    if ui_is_english(texts) {
        return format!(
            "Working sets handled {}, scanned {}, kept/skipped {}, failed {}.",
            report.trimmed_count, report.scanned_count, report.skipped_count, report.failed_count
        );
    }
    format!(
        "工作集处理 {} 个，扫描 {} 个，保留/跳过 {} 个，失败 {} 个。",
        report.trimmed_count, report.scanned_count, report.skipped_count, report.failed_count
    )
}

fn system_release_value_text(report: &OptimizeReport, texts: &UiTexts) -> String {
    if report.dry_run {
        return if ui_is_english(texts) {
            "Preview".to_owned()
        } else {
            "预览".to_owned()
        };
    } else if report.effect.system_cleanup_executed {
        return if ui_is_english(texts) {
            "Released".to_owned()
        } else {
            "已释放".to_owned()
        };
    } else if report.effect.system_cleanup_skipped_reason.is_some() {
        return if ui_is_english(texts) {
            "Checked".to_owned()
        } else {
            "已检查".to_owned()
        };
    }
    if ui_is_english(texts) {
        "No visible change".to_owned()
    } else {
        "无可见变化".to_owned()
    }
}

fn system_release_detail_text(report: &OptimizeReport, texts: &UiTexts) -> String {
    if report.dry_run {
        return if ui_is_english(texts) {
            "Dry run does not release system working sets, standby lists, or candidate processes."
                .to_owned()
        } else {
            "预览模式不执行系统工作集、Standby List 或候选进程释放。".to_owned()
        };
    }

    let delta = report
        .effect
        .standby_cleanup_delta_bytes
        .map(format_signed_bytes)
        .unwrap_or_else(|| {
            if ui_is_english(texts) {
                "no available-memory delta recorded".to_owned()
            } else {
                "无可用内存变化记录".to_owned()
            }
        });
    match &report.effect.system_cleanup_skipped_reason {
        Some(reason) if !reason.is_empty() => {
            let reason = localized_system_release_reason(reason, texts);
            if ui_is_english(texts) {
                format!("{delta}; {reason}")
            } else {
                format!("{delta}；{reason}")
            }
        }
        _ if report.effect.system_cleanup_executed => {
            if ui_is_english(texts) {
                format!("{delta}; system release produced a visible effect.")
            } else {
                format!("{delta}；系统释放步骤已产生可见效果。")
            }
        }
        _ => {
            if ui_is_english(texts) {
                "No visible system-release effect was recorded after working-set handling."
                    .to_owned()
            } else {
                "工作集处理后未记录到系统释放的可见变化。".to_owned()
            }
        }
    }
}

fn localized_system_release_reason(reason: &str, texts: &UiTexts) -> String {
    if !ui_is_english(texts) {
        return reason.to_owned();
    }

    let mut output = reason.to_owned();
    for (source, replacement) in [
        ("内存优化目标释放失败：", "target release failed: "),
        (
            "advanced_cleanup 未启用，已跳过系统级清理",
            "advanced_cleanup is disabled; system cleanup was skipped",
        ),
        (
            "dry-run 只预览，不执行系统级清理",
            "dry run previews only; system cleanup was not executed",
        ),
        (
            "需要管理员权限，已跳过 standby list / 系统工作集清理",
            "administrator permission required; standby list and system working-set cleanup were skipped",
        ),
        ("启用系统级清理权限失败：", "failed to enable system cleanup privilege: "),
        ("系统工作集清理失败：", "system working-set cleanup failed: "),
        (
            "低优先级 standby list 清理失败：",
            "low-priority standby list cleanup failed: ",
        ),
        ("standby list 清理失败：", "standby list cleanup failed: "),
        (
            "modified page list 清理已预留，本版本默认不执行",
            "modified page list cleanup is reserved and not executed by default in this version",
        ),
        (
            "目标释放已进入终止阶段：候选 ",
            "target release entered termination stage: candidates ",
        ),
        ("，温和关闭 ", ", graceful close "),
        ("，强制终止 ", ", force kill "),
        ("，失败 ", ", failed "),
    ] {
        output = output.replace(source, replacement);
    }
    output.replace('；', "; ").replace('，', ", ")
}

fn top_activity_text(report: &OptimizeReport, texts: &UiTexts) -> String {
    if report.dry_run {
        return will_trim_text(report, texts);
    }
    top_trimmed_text(report, texts)
}

fn top_trimmed_text(report: &OptimizeReport, texts: &UiTexts) -> String {
    let top = top_trimmed(report, 8);
    if top.is_empty() {
        return if ui_is_english(texts) {
            "No process was handled in this run. Candidates may have been kept by system, foreground, or activity rules."
                .to_owned()
        } else {
            "本次没有实际处理进程。可能是候选进程较少，或大多被系统边界、前台规则、活跃度规则保留。"
                .to_owned()
        };
    }
    let mut output = String::new();
    let status_label = if ui_is_english(texts) {
        "Status"
    } else {
        "状态"
    };
    let handled = if ui_is_english(texts) {
        "Handled"
    } else {
        "已处理"
    };
    for result in top {
        output.push_str(&format!(
            "{}    {} -> {}    {}: {}    {status_label}: {handled}\n",
            result.name,
            format_working_set_ui(result.before_working_set),
            format_optional_working_set_ui(result.after_working_set, texts),
            texts.reason,
            texts.reason_background_large_ws
        ));
    }
    output
}

fn will_trim_text(report: &OptimizeReport, texts: &UiTexts) -> String {
    let rows: Vec<&TrimResult> = report
        .results
        .iter()
        .filter(|result| matches!(result.status, TrimStatus::WouldTrim))
        .take(8)
        .collect();
    if rows.is_empty() {
        return if ui_is_english(texts) {
            "Dry-run found no handleable processes. Most processes may be kept by system, foreground, or working-set threshold rules."
                .to_owned()
        } else {
            "预览结果中没有可处理进程。多数进程可能已被系统边界、前台规则或工作集阈值保留。"
                .to_owned()
        };
    }
    let mut output = String::new();
    if ui_is_english(texts) {
        output.push_str(
            "Dry run does not optimize memory. These processes would be handled in a real run:\n",
        );
    } else {
        output.push_str("预览模式不会执行内存优化；以下进程在正式执行时将作为工作集处理候选：\n");
    }
    let status_label = if ui_is_english(texts) {
        "Status"
    } else {
        "状态"
    };
    let would_handle = if ui_is_english(texts) {
        "Would handle"
    } else {
        "预计处理"
    };
    for result in rows {
        output.push_str(&format!(
            "{}    {}    {}: {}    {status_label}: {would_handle}\n",
            result.name,
            format_working_set_ui(result.before_working_set),
            texts.reason,
            texts.reason_background_large_ws
        ));
    }
    output
}

fn report_conclusion_text(report: &OptimizeReport, texts: &UiTexts) -> String {
    if report.dry_run {
        let will_trim = report
            .results
            .iter()
            .filter(|result| matches!(result.status, TrimStatus::WouldTrim))
            .count();
        if ui_is_english(texts) {
            return format!(
                "{}: no memory is released. A real run will work toward the <= 25% memory-load target; this preview would handle {} background candidates and keep/skip {} processes.",
                texts.dry_run, will_trim, report.skipped_count
            );
        }
        return format!(
            "这是{}：不会实际释放内存。正式执行时会围绕“内存占用不高于 25%”处理；当前预览将处理 {} 个后台候选进程，保留/跳过 {} 个进程。",
            texts.dry_run,
            will_trim,
            report.skipped_count
        );
    }

    let before_load = memory_load_percent(&report.before);
    let after_load = memory_load_percent(&report.after);
    let change = format_signed_bytes(report.available_increase());
    let system_release = system_release_value_text(report, texts);

    if ui_is_english(texts) {
        if target_reached(report) {
            return format!(
                "Optimization completed: memory load changed from {:.1}% to {:.1}%, reaching the <= 25% target; available memory changed by {}. {} System release: {}.",
                before_load,
                after_load,
                change,
                working_set_result_detail(report, texts),
                system_release
            );
        }
        return format!(
            "All available optimization steps were run: memory load changed from {:.1}% to {:.1}%, but the <= 25% target was not reached; available memory changed by {}. {} System release: {}. Remaining usage usually comes from system reservations, drivers, kernel memory, foreground apps, or processes that cannot be handled.",
            before_load,
            after_load,
            change,
            working_set_result_detail(report, texts),
            system_release
        );
    }

    let target = if target_reached(report) {
        "已达到"
    } else {
        "未达到"
    };

    if target_reached(report) {
        format!(
            "本次优化已完成：内存占用从 {:.1}% 变为 {:.1}%，{}不高于 25% 的目标；可用内存变化 {}。{}系统释放：{}。",
            before_load,
            after_load,
            target,
            change,
            working_set_result_detail(report, texts),
            system_release
        )
    } else {
        format!(
            "本次优化已执行全部可用步骤：内存占用从 {:.1}% 变为 {:.1}%，{}不高于 25% 的目标；可用内存变化 {}。{}系统释放：{}。剩余占用通常来自系统保留、驱动、内核、前台程序或不可处理进程。",
            before_load,
            after_load,
            target,
            change,
            working_set_result_detail(report, texts),
            system_release
        )
    }
}

fn available_change_detail_text(report: &OptimizeReport, texts: &UiTexts) -> String {
    let before_available = gb(report.before.available_physical);
    let after_available = gb(report.after.available_physical);
    let change = format_signed_bytes(report.available_increase());
    let before_load = memory_load_percent(&report.before);
    let after_load = memory_load_percent(&report.after);
    if report.dry_run {
        if ui_is_english(texts) {
            return format!(
                "{}: {}; {}: {}; change {}. {} does not release memory, so +0.0 GB is expected.",
                texts.before, before_available, texts.after, after_available, change, texts.dry_run
            );
        }
        return format!(
            "{}：{}；{}：{}；变化 {}。{}不会实际释放内存，所以 +0.0 GB 属于正常结果。",
            texts.before, before_available, texts.after, after_available, change, texts.dry_run
        );
    }
    let explanation = if ui_is_english(texts) {
        if report.available_increase().abs() < 64 * 1024 * 1024 {
            "The change may display as +0.0 GB when it is below display precision or cache is quickly reallocated."
        } else {
            "This value comes from before/after GlobalMemoryStatusEx and GetPerformanceInfo snapshots."
        }
    } else if report.available_increase().abs() < 64 * 1024 * 1024 {
        "变化小于显示精度或系统很快重新分配缓存时，界面可能显示为 +0.0 GB。"
    } else {
        "该数值来自优化前后 GlobalMemoryStatusEx / GetPerformanceInfo 的快照差异。"
    };
    if ui_is_english(texts) {
        return format!(
            "Load {:.1}% -> {:.1}%; {}: {}; {}: {}; change {}. {}",
            before_load,
            after_load,
            texts.before,
            before_available,
            texts.after,
            after_available,
            change,
            explanation
        );
    }
    format!(
        "占用率 {:.1}% -> {:.1}%；{}：{}；{}：{}；变化 {}。{}",
        before_load,
        after_load,
        texts.before,
        before_available,
        texts.after,
        after_available,
        change,
        explanation
    )
}

fn skip_summary_text(report: &OptimizeReport, texts: &UiTexts) -> String {
    let counts = skip_reason_counts_explained(report, texts);
    if counts.is_empty() {
        return if ui_is_english(texts) {
            "No skipped-reason records.".to_owned()
        } else {
            "没有跳过原因记录。".to_owned()
        };
    }
    let mut output = String::new();
    for (label, (count, explanation)) in counts {
        output.push_str(&format!("{label}: {count} - {explanation}\n"));
    }
    output
}

fn skip_reason_counts_explained(
    report: &OptimizeReport,
    texts: &UiTexts,
) -> BTreeMap<String, (usize, String)> {
    let mut counts = BTreeMap::new();
    for result in &report.results {
        if let TrimStatus::Skipped(reason) = &result.status {
            let label = skip_reason_label(reason, texts);
            let explanation = skip_reason_explanation(reason, texts);
            let entry = counts.entry(label).or_insert((0, explanation));
            entry.0 += 1;
        }
    }
    counts
}

fn skip_reason_explanation(reason: &SkipReason, texts: &UiTexts) -> String {
    if ui_is_english(texts) {
        return match reason {
            SkipReason::SystemProcess => "System stability boundary; kept by default.".to_owned(),
            SkipReason::SelfProcess => "The MemSpark process is not handled.".to_owned(),
            SkipReason::ForegroundProcess => {
                "The current foreground process is skipped to avoid disrupting active work."
                    .to_owned()
            }
            SkipReason::SystemBoundaryProcess => {
                "Matched a hard system boundary; this version does not handle it.".to_owned()
            }
            SkipReason::LegacyRecommendedRule => {
                "Compatibility entry from older reports; that rule module has been removed."
                    .to_owned()
            }
            SkipReason::LegacyUserRule => {
                "Compatibility entry from older reports; user-rule handling has been removed."
                    .to_owned()
            }
            SkipReason::WorkingSetTooSmall => {
                "Working set is below the handling floor, so release benefit is limited.".to_owned()
            }
            SkipReason::AccessDenied => {
                "Windows denied access; the run continued with other processes.".to_owned()
            }
            SkipReason::OpenProcessFailed => {
                "The process could not be opened; it may have exited or denied access.".to_owned()
            }
            SkipReason::QueryFailed => {
                "Process memory information could not be queried.".to_owned()
            }
            SkipReason::ProcessExited => {
                "The process exited after enumeration; this is a normal race.".to_owned()
            }
            SkipReason::HighCpuUsage => {
                "High CPU activity is skipped to avoid disturbing active tasks.".to_owned()
            }
            SkipReason::HighIoActivity => {
                "High I/O activity is skipped to avoid disturbing active reads or writes."
                    .to_owned()
            }
            SkipReason::Other(message) if message.is_empty() => {
                "Other safety or system reason.".to_owned()
            }
            SkipReason::Other(message) => message.clone(),
        };
    }
    match reason {
        SkipReason::SystemProcess => "系统稳定性边界，默认不整理。".to_owned(),
        SkipReason::SelfProcess => "MemSpark 自身进程不会被整理。".to_owned(),
        SkipReason::ForegroundProcess => {
            "当前前台进程默认跳过，避免影响正在使用的程序。".to_owned()
        }
        SkipReason::SystemBoundaryProcess => "命中系统硬边界，当前版本不处理。".to_owned(),
        SkipReason::LegacyRecommendedRule => {
            "旧报告中的推荐规则兼容项；当前版本不再提供该规则模块。".to_owned()
        }
        SkipReason::LegacyUserRule => {
            "旧报告中的用户规则兼容项；当前版本不再提供该规则模块。".to_owned()
        }
        SkipReason::WorkingSetTooSmall => "工作集低于当前处理下限，释放收益有限。".to_owned(),
        SkipReason::AccessDenied => "Windows 权限不足，已记录并继续处理其他进程。".to_owned(),
        SkipReason::OpenProcessFailed => "进程无法打开，可能已退出或权限不足。".to_owned(),
        SkipReason::QueryFailed => "进程内存信息查询失败。".to_owned(),
        SkipReason::ProcessExited => "枚举后进程退出，属于正常竞争情况。".to_owned(),
        SkipReason::HighCpuUsage => "高 CPU 活跃进程会跳过，避免整理活跃任务。".to_owned(),
        SkipReason::HighIoActivity => "高 I/O 活跃进程会跳过，避免影响正在读写的任务。".to_owned(),
        SkipReason::Other(message) if message.is_empty() => "其他安全或系统原因。".to_owned(),
        SkipReason::Other(message) => message.clone(),
    }
}

fn format_ui_dry_run(report: &OptimizeReport, texts: &UiTexts) -> String {
    let mut output = String::new();
    if ui_is_english(texts) {
        output.push_str(&format!("{} - 25% target preview\n\n", texts.dry_run));
    } else {
        output.push_str(&format!("{} - 25% 目标预览\n\n", texts.dry_run));
    }

    output.push_str(texts.will_trim);
    output.push_str(":\n");
    let will_trim: Vec<&TrimResult> = report
        .results
        .iter()
        .filter(|result| matches!(result.status, TrimStatus::WouldTrim))
        .collect();
    if will_trim.is_empty() {
        output.push_str(texts.none);
        output.push('\n');
    } else {
        for result in will_trim {
            output.push_str(&format!(
                "{}    {}    {}: {}\n",
                result.name,
                format_working_set_ui(result.before_working_set),
                texts.reason,
                texts.reason_background_large_ws
            ));
        }
    }

    output.push('\n');
    output.push_str(texts.skipped_summary);
    output.push_str(":\n");
    let counts = skip_reason_counts_ui(report, texts);
    if counts.is_empty() {
        output.push_str(texts.none);
        output.push('\n');
    } else {
        for (label, count) in counts {
            output.push_str(&format!("{label}: {count}\n"));
        }
    }
    output.push('\n');
    output.push_str(texts.skipped_details);
    output.push_str(": ");
    output.push_str(texts.use_verbose);
    output.push('\n');
    output
}

fn format_snapshot_text(snapshot: &MemorySnapshot, texts: &UiTexts) -> String {
    let total = snapshot.total_physical.max(1);
    let used_percent = snapshot.used_physical() as f64 * 100.0 / total as f64;
    format!(
        "{}: {} / {} ({:.1}%)\n{}: {}\n{}: {}\n{}: {} / {}\n{}: {}\n",
        texts.current_memory,
        gb(snapshot.used_physical()),
        gb(snapshot.total_physical),
        used_percent,
        texts.available_memory,
        gb(snapshot.available_physical),
        texts.system_cache,
        gb(snapshot.system_cache),
        texts.commit_usage,
        gb(snapshot.commit_total),
        gb(snapshot.commit_limit),
        texts.process_count,
        snapshot.process_count
    )
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

fn skip_reason_counts_ui(report: &OptimizeReport, texts: &UiTexts) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for result in &report.results {
        if let TrimStatus::Skipped(reason) = &result.status {
            *counts.entry(skip_reason_label(reason, texts)).or_insert(0) += 1;
        }
    }
    counts
}

fn skip_reason_label(reason: &SkipReason, texts: &UiTexts) -> String {
    match reason {
        SkipReason::SystemProcess => texts.skip_system_process.to_owned(),
        SkipReason::SelfProcess => texts.skip_self_process.to_owned(),
        SkipReason::ForegroundProcess => texts.skip_foreground_process.to_owned(),
        SkipReason::SystemBoundaryProcess => texts.skip_system_boundary.to_owned(),
        SkipReason::LegacyRecommendedRule => texts.skip_legacy_recommended_rule.to_owned(),
        SkipReason::LegacyUserRule => texts.skip_legacy_user_rule.to_owned(),
        SkipReason::WorkingSetTooSmall => texts.skip_working_set_small.to_owned(),
        SkipReason::AccessDenied => texts.skip_access_denied.to_owned(),
        SkipReason::OpenProcessFailed => texts.skip_open_process_failed.to_owned(),
        SkipReason::QueryFailed => texts.skip_query_failed.to_owned(),
        SkipReason::ProcessExited => texts.skip_process_exited.to_owned(),
        SkipReason::HighCpuUsage => texts.skip_high_cpu.to_owned(),
        SkipReason::HighIoActivity => texts.skip_high_io.to_owned(),
        SkipReason::Other(message) if message.is_empty() => texts.skip_other.to_owned(),
        SkipReason::Other(message) => message.clone(),
    }
}

fn format_working_set_ui(bytes: u64) -> String {
    if bytes == 0 {
        "0 MB".to_owned()
    } else if bytes < 1024 * 1024 {
        "<1 MB".to_owned()
    } else {
        format!("{:.0} MB", bytes as f64 / 1024.0 / 1024.0)
    }
}

fn format_optional_working_set_ui(bytes: Option<u64>, texts: &UiTexts) -> String {
    match bytes {
        Some(bytes) => format_working_set_ui(bytes),
        None => texts.unknown.to_owned(),
    }
}

fn format_signed_bytes(bytes: i64) -> String {
    let sign = if bytes >= 0 { "+" } else { "-" };
    let abs = bytes.unsigned_abs();
    format!("{}{}", sign, gb(abs))
}

fn current_ui_settings(settings: &Arc<Mutex<UiSettings>>) -> UiSettings {
    settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn current_settings_draft(draft: &Arc<Mutex<SettingsDraft>>) -> SettingsDraft {
    draft
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn set_settings_draft(draft: &Arc<Mutex<SettingsDraft>>, value: SettingsDraft) {
    let mut guard = draft
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = value;
}

fn ui_settings_path() -> PathBuf {
    app_data_dir().join("ui_settings.toml")
}

fn load_ui_settings() -> UiSettings {
    let mut settings = UiSettings::default();
    let Ok(content) = fs::read_to_string(ui_settings_path()) else {
        return settings;
    };
    for line in content.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key.trim() {
            "language" => {
                if let Some(language) = parse_ui_language(value) {
                    settings.language = language;
                }
            }
            "theme" => {
                if let Some(theme) = parse_ui_theme(value) {
                    settings.theme = theme.to_owned();
                }
            }
            _ => {}
        }
    }
    settings
}

fn save_ui_settings(settings: &UiSettings) -> Result<PathBuf, String> {
    let path = ui_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let content = format!(
        "language = \"{}\"\ntheme = \"{}\"\n",
        ui_language_code(settings.language),
        settings.theme
    );
    fs::write(&path, content).map_err(|err| err.to_string())?;
    Ok(path)
}

fn load_settings_draft(settings: &UiSettings) -> SettingsDraft {
    match load_config_or_default(None) {
        Ok(config) => SettingsDraft {
            language: settings.language,
            theme: settings.theme.clone(),
            report_path: expand_env_path(&config.report.last_report_path),
            report_history_dir: expand_env_path(&config.report.history_report_dir),
            developer_log_dir: expand_env_path(&config.report.developer_log_dir),
        },
        Err(_) => SettingsDraft {
            language: settings.language,
            theme: settings.theme.clone(),
            report_path: app_data_dir().join("last_report.json"),
            report_history_dir: default_report_history_dir(),
            developer_log_dir: default_developer_log_dir(),
        },
    }
}

fn parse_ui_language(value: &str) -> Option<UiLanguage> {
    match value.trim().to_ascii_lowercase().as_str() {
        "zh" | "zh-cn" | "cn" => Some(UiLanguage::ZhCn),
        "en" | "en-us" => Some(UiLanguage::EnUs),
        _ => None,
    }
}

fn ui_language_code(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::ZhCn => "zh-CN",
        UiLanguage::EnUs => "en-US",
    }
}

fn parse_ui_theme(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "dark" => Some("dark"),
        "light" => Some("light"),
        _ => None,
    }
}

fn refresh_settings(
    app: &AppWindow,
    texts: &'static UiTexts,
    settings_state: &Arc<Mutex<UiSettings>>,
    draft_state: &Arc<Mutex<SettingsDraft>>,
) {
    let settings = current_ui_settings(settings_state);
    match load_config_or_default(None) {
        Ok(config) => {
            let draft = SettingsDraft {
                language: settings.language,
                theme: settings.theme,
                report_path: expand_env_path(&config.report.last_report_path),
                report_history_dir: expand_env_path(&config.report.history_report_dir),
                developer_log_dir: expand_env_path(&config.report.developer_log_dir),
            };
            set_settings_draft(draft_state, draft.clone());
            apply_settings_draft_to_ui(app, &draft, texts);
        }
        Err(err) => {
            let path_message = if ui_is_english(texts) {
                format!("Failed to read report paths: {err}")
            } else {
                format!("报告保存路径读取失败：{err}")
            };
            app.set_report_path_text(path_message.into());
            app.set_log_text(format!("{}{}", texts.config_init_failed, err).into());
        }
    }
}

fn apply_settings_draft_to_ui(app: &AppWindow, draft: &SettingsDraft, texts: &UiTexts) {
    app.set_ui_language(ui_language_code(draft.language).into());
    app.set_ui_theme(draft.theme.clone().into());
    app.set_report_path_text(
        format!(
            "{}: {}\n{}: {}\n{}: {}",
            texts.report_path_last_report,
            draft.report_path.display(),
            texts.report_path_history_dir,
            draft.report_history_dir.display(),
            texts.report_path_developer_logs,
            draft.developer_log_dir.display()
        )
        .into(),
    );
}

fn save_settings_and_restart(
    app: &AppWindow,
    settings_state: &Arc<Mutex<UiSettings>>,
    draft_state: &Arc<Mutex<SettingsDraft>>,
    texts: &UiTexts,
) {
    let draft = current_settings_draft(draft_state);
    let result = (|| {
        let settings = UiSettings {
            language: draft.language,
            theme: draft.theme.clone(),
        };
        save_ui_settings(&settings)?;
        let mut config = load_config_or_default(None).map_err(|err| err.to_string())?;
        config.report.last_report_path = draft.report_path.to_string_lossy().to_string();
        save_config(None, &config).map_err(|err| err.to_string())?;
        Ok::<UiSettings, String>(settings)
    })();

    match result {
        Ok(settings) => {
            {
                let mut guard = settings_state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *guard = settings;
            }
            let message = if ui_is_english(texts) {
                "Settings saved. MemSpark will restart to apply them."
            } else {
                "设置已保存，应用将自动重启以生效。"
            };
            app.set_log_text(message.into());
            match restart_current_app() {
                Ok(()) => {
                    std::thread::spawn(|| {
                        std::thread::sleep(Duration::from_millis(500));
                        std::process::exit(0);
                    });
                }
                Err(err) if ui_is_english(texts) => app.set_log_text(
                    format!("Settings saved, but automatic restart failed: {err}").into(),
                ),
                Err(err) => app.set_log_text(format!("设置已保存，但自动重启失败：{err}").into()),
            }
        }
        Err(err) if ui_is_english(texts) => {
            app.set_log_text(format!("Failed to save settings: {err}").into())
        }
        Err(err) => app.set_log_text(format!("设置保存失败：{err}").into()),
    }
}

fn restart_current_app() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    Command::new(exe)
        .spawn()
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn open_report_path(app: &AppWindow, path: PathBuf, texts: &UiTexts) {
    if path.exists() {
        match open_path(&path) {
            Ok(()) if ui_is_english(texts) => {
                app.set_log_text(format!("Opened report: {}", path.display()).into())
            }
            Ok(()) => app.set_log_text(format!("已打开报告：{}", path.display()).into()),
            Err(err) if ui_is_english(texts) => {
                app.set_log_text(format!("Failed to open report: {err}").into())
            }
            Err(err) => app.set_log_text(format!("打开报告失败：{err}").into()),
        }
    } else {
        let message = if ui_is_english(texts) {
            "No report file yet. Run optimization first."
        } else {
            "暂无报告文件，请先执行一次预览或优化。"
        };
        app.set_log_text(message.into());
    }
}

fn open_report_folder_path(app: &AppWindow, path: PathBuf, texts: &UiTexts) {
    let folder = path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(app_data_dir);
    match open_path(&folder) {
        Ok(()) if ui_is_english(texts) => {
            app.set_log_text(format!("Opened report folder: {}", folder.display()).into())
        }
        Ok(()) => app.set_log_text(format!("已打开报告文件夹：{}", folder.display()).into()),
        Err(err) if ui_is_english(texts) => {
            app.set_log_text(format!("Failed to open report folder: {err}").into())
        }
        Err(err) => app.set_log_text(format!("打开报告文件夹失败：{err}").into()),
    }
}

fn open_path(path: &PathBuf) -> Result<(), String> {
    Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn choose_report_folder_async(
    weak: slint::Weak<AppWindow>,
    draft_state: Arc<Mutex<SettingsDraft>>,
    texts: &'static UiTexts,
) {
    if let Some(app) = weak.upgrade() {
        let message = if ui_is_english(texts) {
            "Choose a report save folder."
        } else {
            "请选择报告保存文件夹。"
        };
        app.set_log_text(message.into());
    }
    std::thread::spawn(move || {
        let dialog_title = if ui_is_english(texts) {
            "Choose MemSpark report folder"
        } else {
            "选择 MemSpark 报告保存文件夹"
        };
        let script = format!("Add-Type -AssemblyName System.Windows.Forms; $dialog = New-Object System.Windows.Forms.FolderBrowserDialog; $dialog.Description = '{}'; if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {{ $dialog.SelectedPath }}", dialog_title);
        let selected = Command::new("powershell.exe")
            .args(["-NoProfile", "-STA", "-Command", &script])
            .output()
            .map_err(|err| err.to_string())
            .and_then(|output| {
                if output.status.success() {
                    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
                } else {
                    Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
                }
            });
        let result = selected.map(|folder| {
            if folder.is_empty() {
                return None;
            }
            let path = PathBuf::from(folder).join("last_report.json");
            Some(path)
        });
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                match result {
                    Ok(Some(path)) => {
                        let mut draft = current_settings_draft(&draft_state);
                        draft.report_path = path.clone();
                        set_settings_draft(&draft_state, draft.clone());
                        apply_settings_draft_to_ui(&app, &draft, texts);
                        let message = if ui_is_english(texts) {
                            format!(
                                "Report save path selected: {}; waiting to save.",
                                path.display()
                            )
                        } else {
                            format!("报告保存位置已选择：{}，等待保存。", path.display())
                        };
                        app.set_log_text(message.into());
                    }
                    Ok(None) if ui_is_english(texts) => {
                        app.set_log_text("Folder selection canceled.".into())
                    }
                    Ok(None) => app.set_log_text("已取消选择保存位置。".into()),
                    Err(err) if ui_is_english(texts) => {
                        app.set_log_text(format!("Failed to choose save path: {err}").into())
                    }
                    Err(err) => app.set_log_text(format!("选择保存位置失败：{err}").into()),
                }
            }
        });
    });
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
}

fn format_capacity_gb(bytes: u64) -> String {
    let gb = bytes as f64 / 1024.0 / 1024.0 / 1024.0;
    if (gb - gb.round()).abs() < 0.05 {
        format!("{gb:.0} GB")
    } else {
        format!("{gb:.1} GB")
    }
}

fn ratio(value: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (value as f64 / total as f64).clamp(0.0, 1.0) as f32
    }
}
