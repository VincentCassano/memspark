use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptimizeMode {
    #[serde(rename = "memory_optimization", alias = "normal", alias = "strong")]
    MemoryOptimization,
}

impl OptimizeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MemoryOptimization => "memory_optimization",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::MemoryOptimization => "内存优化",
        }
    }
}

impl fmt::Display for OptimizeMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

impl FromStr for OptimizeMode {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "memory"
            | "memory_optimization"
            | "memory-optimization"
            | "optimize"
            | "optimization"
            | "内存优化"
            | "normal"
            | "strong" => Ok(Self::MemoryOptimization),
            other => Err(format!("unsupported optimize mode: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrimMethod {
    #[default]
    EmptyWorkingSet,
    SetProcessWorkingSetSizeEx,
}

impl fmt::Display for TrimMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyWorkingSet => f.write_str("EmptyWorkingSet"),
            Self::SetProcessWorkingSetSizeEx => f.write_str("SetProcessWorkingSetSizeEx"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModePolicy {
    pub min_working_set_mb: u64,
    pub skip_foreground: bool,
    pub skip_system_process: bool,
    pub skip_self: bool,
    pub skip_high_cpu: bool,
    pub high_cpu_threshold_percent: u32,
    pub skip_high_io: bool,
    pub max_rounds: u32,
    pub round_delay_ms: u64,
    pub require_admin: bool,
    pub trim_method: TrimMethod,
    #[serde(default)]
    pub allow_trim_explorer: bool,
    #[serde(default)]
    pub allow_trim_browser: bool,
}

impl ModePolicy {
    pub fn memory_optimization() -> Self {
        Self {
            min_working_set_mb: 10,
            skip_foreground: true,
            skip_system_process: true,
            skip_self: true,
            skip_high_cpu: true,
            high_cpu_threshold_percent: 100,
            skip_high_io: true,
            max_rounds: 5,
            round_delay_ms: 150,
            require_admin: true,
            trim_method: TrimMethod::EmptyWorkingSet,
            allow_trim_explorer: false,
            allow_trim_browser: true,
        }
    }

    pub fn min_working_set_bytes(&self) -> u64 {
        self.min_working_set_mb.saturating_mul(1024 * 1024)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkipReason {
    SystemProcess,
    SelfProcess,
    ForegroundProcess,
    SystemBoundaryProcess,
    LegacyRecommendedRule,
    LegacyUserRule,
    WorkingSetTooSmall,
    AccessDenied,
    OpenProcessFailed,
    QueryFailed,
    ProcessExited,
    HighCpuUsage,
    HighIoActivity,
    Other(String),
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SystemProcess => f.write_str("system process"),
            Self::SelfProcess => f.write_str("self process"),
            Self::ForegroundProcess => f.write_str("foreground process"),
            Self::SystemBoundaryProcess => f.write_str("system boundary process"),
            Self::LegacyRecommendedRule => f.write_str("legacy recommended rule"),
            Self::LegacyUserRule => f.write_str("legacy user rule"),
            Self::WorkingSetTooSmall => f.write_str("working set below policy threshold"),
            Self::AccessDenied => f.write_str("access denied"),
            Self::OpenProcessFailed => f.write_str("OpenProcess failed"),
            Self::QueryFailed => f.write_str("process query failed"),
            Self::ProcessExited => f.write_str("process exited"),
            Self::HighCpuUsage => f.write_str("high CPU usage"),
            Self::HighIoActivity => f.write_str("high I/O activity"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrimStatus {
    WouldTrim,
    Trimmed,
    Skipped(SkipReason),
    Failed(String),
}

impl fmt::Display for TrimStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WouldTrim => f.write_str("would trim"),
            Self::Trimmed => f.write_str("trimmed"),
            Self::Skipped(reason) => write!(f, "skipped: {reason}"),
            Self::Failed(message) => write!(f, "failed: {message}"),
        }
    }
}

pub fn system_boundary_processes() -> &'static [&'static str] {
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
    ]
}

pub fn browser_processes() -> &'static [&'static str] {
    &[
        "chrome.exe",
        "chromium.exe",
        "msedge.exe",
        "msedgewebview2.exe",
        "firefox.exe",
        "brave.exe",
        "opera.exe",
        "vivaldi.exe",
        "qqbrowser.exe",
        "360chrome.exe",
        "360se.exe",
        "sogouexplorer.exe",
    ]
}

pub fn vm_memory_processes() -> &'static [&'static str] {
    &[
        "vmmem",
        "vmmem.exe",
        "vmmemWSL",
        "vmmemWSL.exe",
        "vmmemCmZygote",
        "vmwp.exe",
        "vmms.exe",
        "vmware-vmx.exe",
        "VirtualBoxVM.exe",
    ]
}

pub fn vm_stack_processes() -> &'static [&'static str] {
    &[
        "vmmem",
        "vmmem.exe",
        "vmmemWSL",
        "vmmemWSL.exe",
        "vmmemCmZygote",
        "vmwp.exe",
        "vmms.exe",
        "wsl.exe",
        "wslhost.exe",
        "wslrelay.exe",
        "wslservice.exe",
        "docker-sandbox.exe",
        "Docker Desktop.exe",
        "com.docker.backend.exe",
        "com.docker.build.exe",
        "vmware-vmx.exe",
        "vmware-authd.exe",
        "vmware-usbarbitrator64.exe",
        "vmnat.exe",
        "vmnetdhcp.exe",
        "VirtualBoxVM.exe",
    ]
}

pub fn same_process_name(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}
