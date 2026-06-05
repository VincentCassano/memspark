use crate::policy::ModePolicy;
use crate::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    pub mode: ModeConfig,
    pub report: ReportConfig,
    #[serde(default)]
    pub advanced_cleanup: AdvancedCleanupConfig,
}

impl AppConfig {
    pub fn policy(&self) -> &ModePolicy {
        &self.mode.optimization
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeConfig {
    #[serde(default = "ModePolicy::memory_optimization")]
    pub optimization: ModePolicy,
}

impl Default for ModeConfig {
    fn default() -> Self {
        Self {
            optimization: ModePolicy::memory_optimization(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportConfig {
    pub save_last_report: bool,
    pub last_report_path: String,
    #[serde(default = "default_report_history_dir_string")]
    pub history_report_dir: String,
    #[serde(default = "default_developer_log_dir_string")]
    pub developer_log_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedCleanupConfig {
    pub enable_standby_list_cleanup: bool,
    pub enable_system_working_set_cleanup: bool,
    pub enable_modified_page_list_cleanup: bool,
    pub require_admin_for_system_cleanup: bool,
}

impl Default for AdvancedCleanupConfig {
    fn default() -> Self {
        Self {
            enable_standby_list_cleanup: false,
            enable_system_working_set_cleanup: false,
            enable_modified_page_list_cleanup: false,
            require_admin_for_system_cleanup: true,
        }
    }
}

impl Default for ReportConfig {
    fn default() -> Self {
        Self {
            save_last_report: true,
            last_report_path: "%APPDATA%\\MemSpark\\last_report.json".to_owned(),
            history_report_dir: "%APPDATA%\\MemSpark\\history".to_owned(),
            developer_log_dir: "%APPDATA%\\MemSpark\\logs".to_owned(),
        }
    }
}

fn default_report_history_dir_string() -> String {
    ReportConfig::default().history_report_dir
}

fn default_developer_log_dir_string() -> String {
    ReportConfig::default().developer_log_dir
}

pub fn default_config_path() -> PathBuf {
    app_data_dir().join("config.toml")
}

pub fn default_report_path() -> PathBuf {
    app_data_dir().join("last_report.json")
}

pub fn default_report_history_dir() -> PathBuf {
    app_data_dir().join("history")
}

pub fn default_developer_log_dir() -> PathBuf {
    app_data_dir().join("logs")
}

pub fn app_data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".appdata")
        })
        .join("MemSpark")
}

pub fn load_config_or_default(path: Option<&Path>) -> Result<AppConfig> {
    let path = path.map(PathBuf::from).unwrap_or_else(default_config_path);
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let content = fs::read_to_string(path)?;
    Ok(toml::from_str(&content)?)
}

pub fn save_config(path: Option<&Path>, config: &AppConfig) -> Result<PathBuf> {
    let path = path.map(PathBuf::from).unwrap_or_else(default_config_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(config)?;
    fs::write(&path, content)?;
    Ok(path)
}

pub fn init_config(path: Option<&Path>, force: bool) -> Result<PathBuf> {
    let path = path.map(PathBuf::from).unwrap_or_else(default_config_path);
    if path.exists() && !force {
        return Ok(path);
    }
    save_config(Some(&path), &AppConfig::default())
}

pub fn expand_env_path(path: &str) -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_owned());
    PathBuf::from(path.replace("%APPDATA%", &appdata))
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn memory_optimization_is_the_only_mode_by_default() {
        let config = AppConfig::default();

        assert_eq!(config.mode.optimization.min_working_set_mb, 10);
        assert!(config.mode.optimization.require_admin);
    }
}
