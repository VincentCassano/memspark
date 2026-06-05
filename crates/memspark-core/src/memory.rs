use crate::{winapi, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnapshot {
    pub total_physical: u64,
    pub available_physical: u64,
    pub memory_load_percent: u32,
    pub commit_total: u64,
    pub commit_limit: u64,
    pub system_cache: u64,
    pub process_count: u32,
    pub page_file_total: u64,
    pub page_file_available: u64,
    pub virtual_total: u64,
    pub virtual_available: u64,
}

impl MemorySnapshot {
    pub fn used_physical(&self) -> u64 {
        self.total_physical.saturating_sub(self.available_physical)
    }
}

pub fn get_memory_snapshot() -> Result<MemorySnapshot> {
    winapi::memory_snapshot()
}
