use crate::{winapi, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub path: Option<String>,
    pub working_set: u64,
    pub private_usage: Option<u64>,
    pub is_foreground: bool,
    pub is_self: bool,
    pub query_error: Option<String>,
}

pub fn enumerate_processes() -> Result<Vec<ProcessInfo>> {
    let raw_processes = winapi::enumerate_processes_raw()?;
    let foreground_pid = winapi::foreground_process_id().unwrap_or(None);
    let current_pid = winapi::current_process_id();
    let mut processes = Vec::with_capacity(raw_processes.len());

    for raw in raw_processes {
        let query = winapi::query_process(raw.pid);
        let (path, working_set, private_usage, query_error) = match query {
            Ok(info) => (info.path, info.working_set, info.private_usage, None),
            Err(err) => (None, 0, None, Some(err.to_string())),
        };

        processes.push(ProcessInfo {
            pid: raw.pid,
            name: raw.name,
            path,
            working_set,
            private_usage,
            is_foreground: foreground_pid == Some(raw.pid),
            is_self: raw.pid == current_pid,
            query_error,
        });
    }

    Ok(processes)
}
