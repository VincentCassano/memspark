pub mod config;
pub mod memory;
pub mod optimizer;
pub mod policy;
pub mod process;
pub mod release;
pub mod report;
pub mod termination;
pub mod winapi;

pub use config::{
    default_config_path, default_report_path, AdvancedCleanupConfig, AppConfig, ModeConfig,
    ReportConfig,
};
pub use memory::{get_memory_snapshot, MemorySnapshot};
pub use optimizer::{
    optimize, trim_working_sets_aggressive, AggressiveTrimOptions, AggressiveTrimReport,
};
pub use policy::{
    system_boundary_processes, vm_memory_processes, vm_stack_processes, ModePolicy, OptimizeMode,
    SkipReason, TrimMethod, TrimStatus,
};
pub use process::{enumerate_processes, ProcessInfo};
pub use release::{
    run_system_release, run_targeted_system_release, SystemReleaseReport, SystemReleaseStep,
    SystemReleaseStepResult, SystemReleaseStepStatus,
};
pub use report::{
    clear_developer_logs, clear_report_history, format_report_text,
    format_report_text_with_options, load_last_report, save_developer_log, save_report,
    CleanupEffectSummary, OptimizeReport, TrimResult,
};
pub use termination::{
    run_process_release_all, run_targeted_process_release, ProcessReleaseReport,
    ProcessReleaseResult, ProcessReleaseSkipReason, ProcessReleaseStatus,
};
pub use winapi::{
    enable_debug_privilege, enable_increase_quota_privilege, enable_profile_privilege,
    is_running_as_admin, run_elevated_and_wait,
};

#[derive(Debug, thiserror::Error)]
pub enum MemSparkError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("TOML serialize error: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Windows API error: {0}")]
    WinApi(String),
    #[error("configuration error: {0}")]
    Config(String),
    #[error("unsupported on this platform: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, MemSparkError>;
