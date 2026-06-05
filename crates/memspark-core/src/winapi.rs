use std::fmt;

#[derive(Debug, Clone)]
pub struct RawProcessInfo {
    pub pid: u32,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ProcessQueryInfo {
    pub path: Option<String>,
    pub working_set: u64,
    pub private_usage: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct WinApiError {
    pub context: &'static str,
    pub code: u32,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub enum MemoryListCommand {
    EmptyWorkingSets,
    FlushModifiedList,
    PurgeStandbyList,
    PurgeLowPriorityStandbyList,
}

impl WinApiError {
    pub fn is_access_denied(&self) -> bool {
        self.code == 5
    }

    pub fn is_process_exited(&self) -> bool {
        self.code == 87 || self.code == 1168 || self.code == 18
    }
}

impl fmt::Display for WinApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} failed with Windows error {} ({})",
            self.context, self.code, self.message
        )
    }
}

#[cfg(windows)]
mod imp {
    use super::{MemoryListCommand, ProcessQueryInfo, RawProcessInfo, WinApiError};
    use crate::{memory::MemorySnapshot, MemSparkError, Result};
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, BOOL, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, LUID,
    };
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, GetTokenInformation, LookupPrivilegeValueW, TokenElevation,
        LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION,
        TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Memory::SetProcessWorkingSetSizeEx;
    use windows_sys::Win32::System::ProcessStatus::{
        EmptyWorkingSet, GetPerformanceInfo, K32GetProcessImageFileNameW, K32GetProcessMemoryInfo,
        PERFORMANCE_INFORMATION, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, GetExitCodeProcess, OpenProcess, OpenProcessToken,
        TerminateProcess, WaitForSingleObject, PROCESS_QUERY_INFORMATION,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE, PROCESS_VM_READ,
    };
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible, PostMessageW,
        SW_HIDE, WM_CLOSE,
    };

    struct Handle(HANDLE);

    const SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;
    const ERROR_NOT_ALL_ASSIGNED: u32 = 1300;
    const SYSTEM_MEMORY_LIST_INFORMATION: u32 = 80;
    const STATUS_SUCCESS: i32 = 0;
    const STATUS_PRIVILEGE_NOT_HELD: i32 = 0xC000_0061u32 as i32;
    const STATUS_ACCESS_DENIED: i32 = 0xC000_0022u32 as i32;
    const INFINITE: u32 = 0xffff_ffff;

    #[link(name = "ntdll")]
    extern "system" {
        fn NtSetSystemInformation(
            system_information_class: u32,
            system_information: *mut c_void,
            system_information_length: u32,
        ) -> i32;
    }

    impl Handle {
        fn is_invalid(&self) -> bool {
            self.0.is_null() || self.0 == INVALID_HANDLE_VALUE
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.is_invalid() {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    pub fn current_process_id() -> u32 {
        unsafe { GetCurrentProcessId() }
    }

    pub fn foreground_process_id() -> Result<Option<u32>> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return Ok(None);
            }
            let mut pid = 0u32;
            let _thread_id = GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == 0 {
                Ok(None)
            } else {
                Ok(Some(pid))
            }
        }
    }

    pub fn memory_snapshot() -> Result<MemorySnapshot> {
        unsafe {
            let mut memory: MEMORYSTATUSEX = zeroed();
            memory.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
            if GlobalMemoryStatusEx(&mut memory) == 0 {
                return Err(MemSparkError::WinApi(
                    last_error("GlobalMemoryStatusEx").to_string(),
                ));
            }

            let mut perf: PERFORMANCE_INFORMATION = zeroed();
            perf.cb = size_of::<PERFORMANCE_INFORMATION>() as u32;
            if GetPerformanceInfo(&mut perf, size_of::<PERFORMANCE_INFORMATION>() as u32) == 0 {
                return Err(MemSparkError::WinApi(
                    last_error("GetPerformanceInfo").to_string(),
                ));
            }

            let page_size = perf.PageSize as u64;
            Ok(MemorySnapshot {
                total_physical: memory.ullTotalPhys,
                available_physical: memory.ullAvailPhys,
                memory_load_percent: memory.dwMemoryLoad,
                commit_total: (perf.CommitTotal as u64).saturating_mul(page_size),
                commit_limit: (perf.CommitLimit as u64).saturating_mul(page_size),
                system_cache: (perf.SystemCache as u64).saturating_mul(page_size),
                process_count: perf.ProcessCount,
                page_file_total: memory.ullTotalPageFile,
                page_file_available: memory.ullAvailPageFile,
                virtual_total: memory.ullTotalVirtual,
                virtual_available: memory.ullAvailVirtual,
            })
        }
    }

    pub fn enumerate_processes_raw() -> Result<Vec<RawProcessInfo>> {
        unsafe {
            let snapshot = Handle(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0));
            if snapshot.is_invalid() {
                return Err(MemSparkError::WinApi(
                    last_error("CreateToolhelp32Snapshot").to_string(),
                ));
            }

            let mut entry: PROCESSENTRY32W = zeroed();
            entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
            if Process32FirstW(snapshot.0, &mut entry) == 0 {
                return Err(MemSparkError::WinApi(
                    last_error("Process32FirstW").to_string(),
                ));
            }

            let mut processes = Vec::new();
            loop {
                processes.push(RawProcessInfo {
                    pid: entry.th32ProcessID,
                    name: wide_array_to_string(&entry.szExeFile),
                });

                if Process32NextW(snapshot.0, &mut entry) == 0 {
                    break;
                }
            }
            Ok(processes)
        }
    }

    pub fn query_process(pid: u32) -> std::result::Result<ProcessQueryInfo, WinApiError> {
        let handle = open_process_for_query(pid)?;
        let memory = match process_memory_from_handle(handle.0) {
            Ok(memory) => memory,
            Err(_) => {
                let handle = open_process(
                    pid,
                    PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
                    "OpenProcess(query memory)",
                )
                .or_else(|_| open_process(pid, PROCESS_QUERY_INFORMATION, "OpenProcess(query)"))?;
                process_memory_from_handle(handle.0)?
            }
        };
        let path = process_image_path(handle.0).ok();
        Ok(ProcessQueryInfo {
            path,
            working_set: memory.0,
            private_usage: memory.1,
        })
    }

    pub fn query_process_working_set(pid: u32) -> std::result::Result<u64, WinApiError> {
        let handle = open_process_for_query(pid)?;
        let memory = match process_memory_from_handle(handle.0) {
            Ok(memory) => memory,
            Err(_) => {
                let handle = open_process(
                    pid,
                    PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
                    "OpenProcess(query working set memory)",
                )
                .or_else(|_| {
                    open_process(
                        pid,
                        PROCESS_QUERY_INFORMATION,
                        "OpenProcess(query working set)",
                    )
                })?;
                process_memory_from_handle(handle.0)?
            }
        };
        Ok(memory.0)
    }

    pub fn trim_process_empty_working_set(pid: u32) -> std::result::Result<(), WinApiError> {
        let handle = open_process(
            pid,
            PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION,
            "OpenProcess(trim)",
        )
        .or_else(|_| {
            open_process(
                pid,
                PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION,
                "OpenProcess(trim limited)",
            )
        })
        .or_else(|_| {
            open_process(
                pid,
                PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION,
                "OpenProcess(trim compatibility)",
            )
        })
        .or_else(|_| open_process(pid, PROCESS_SET_QUOTA, "OpenProcess(trim quota only)"))?;
        unsafe {
            if EmptyWorkingSet(handle.0) == 0 {
                return Err(last_error("EmptyWorkingSet"));
            }
        }
        Ok(())
    }

    pub fn trim_process_set_working_set_size_ex(pid: u32) -> std::result::Result<(), WinApiError> {
        let handle = open_process(
            pid,
            PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION,
            "OpenProcess(trim fallback)",
        )
        .or_else(|_| {
            open_process(
                pid,
                PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION,
                "OpenProcess(trim fallback limited)",
            )
        })
        .or_else(|_| {
            open_process(
                pid,
                PROCESS_SET_QUOTA,
                "OpenProcess(trim fallback quota only)",
            )
        })?;
        unsafe {
            if SetProcessWorkingSetSizeEx(handle.0, usize::MAX, usize::MAX, 0) == 0 {
                return Err(last_error("SetProcessWorkingSetSizeEx"));
            }
        }
        Ok(())
    }

    pub fn graceful_close_process_windows(pid: u32) -> std::result::Result<u32, WinApiError> {
        struct EnumState {
            pid: u32,
            posted: u32,
        }

        unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let state = &mut *(lparam as *mut EnumState);
            if IsWindowVisible(hwnd) == 0 {
                return 1;
            }
            let mut window_pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut window_pid);
            if window_pid == state.pid && PostMessageW(hwnd, WM_CLOSE, 0, 0) != 0 {
                state.posted = state.posted.saturating_add(1);
            }
            1
        }

        let mut state = EnumState { pid, posted: 0 };
        unsafe {
            if EnumWindows(Some(enum_proc), &mut state as *mut EnumState as LPARAM) == 0 {
                return Err(last_error("EnumWindows"));
            }
        }
        Ok(state.posted)
    }

    pub fn wait_for_process_exit(
        pid: u32,
        timeout_ms: u32,
    ) -> std::result::Result<bool, WinApiError> {
        let handle = open_process(pid, SYNCHRONIZE_ACCESS, "OpenProcess(wait)")?;
        let result = unsafe { WaitForSingleObject(handle.0, timeout_ms) };
        match result {
            0 => Ok(true),
            0x0000_0102 => Ok(false),
            0xffff_ffff => Err(last_error("WaitForSingleObject")),
            _ => Ok(false),
        }
    }

    pub fn terminate_process(pid: u32) -> std::result::Result<(), WinApiError> {
        let handle = open_process(pid, PROCESS_TERMINATE, "OpenProcess(terminate)")?;
        unsafe {
            if TerminateProcess(handle.0, 1) == 0 {
                return Err(last_error("TerminateProcess"));
            }
        }
        Ok(())
    }

    pub fn is_running_as_admin() -> bool {
        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let token = Handle(token);

            let mut elevation: TOKEN_ELEVATION = zeroed();
            let mut returned = 0u32;
            if GetTokenInformation(
                token.0,
                TokenElevation,
                &mut elevation as *mut TOKEN_ELEVATION as *mut c_void,
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            ) == 0
            {
                return false;
            }

            elevation.TokenIsElevated != 0
        }
    }

    pub fn enable_profile_privilege() -> std::result::Result<(), WinApiError> {
        enable_privilege(
            "SeProfileSingleProcessPrivilege",
            "SeProfileSingleProcessPrivilege",
        )
    }

    pub fn enable_debug_privilege() -> std::result::Result<(), WinApiError> {
        enable_privilege("SeDebugPrivilege", "SeDebugPrivilege")
    }

    pub fn enable_increase_quota_privilege() -> std::result::Result<(), WinApiError> {
        enable_privilege("SeIncreaseQuotaPrivilege", "SeIncreaseQuotaPrivilege")
    }

    pub fn run_elevated_and_wait(
        executable: &str,
        parameters: &str,
    ) -> std::result::Result<u32, WinApiError> {
        let verb = wide_null("runas");
        let file = wide_null(executable);
        let params = wide_null(parameters);
        let mut info: SHELLEXECUTEINFOW = unsafe { zeroed() };
        info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_HIDE;

        unsafe {
            if ShellExecuteExW(&mut info) == 0 {
                return Err(last_error("ShellExecuteExW(runas)"));
            }
            let handle = Handle(info.hProcess);
            if handle.is_invalid() {
                return Ok(0);
            }
            let wait = WaitForSingleObject(handle.0, INFINITE);
            if wait == 0xffff_ffff {
                return Err(last_error("WaitForSingleObject(elevated)"));
            }
            let mut exit_code = 0u32;
            if GetExitCodeProcess(handle.0, &mut exit_code) == 0 {
                return Err(last_error("GetExitCodeProcess(elevated)"));
            }
            Ok(exit_code)
        }
    }

    pub fn set_system_memory_list(
        command: MemoryListCommand,
    ) -> std::result::Result<(), WinApiError> {
        let mut command = memory_list_command_value(command);
        let status = unsafe {
            NtSetSystemInformation(
                SYSTEM_MEMORY_LIST_INFORMATION,
                &mut command as *mut i32 as *mut c_void,
                size_of::<i32>() as u32,
            )
        };
        if status == STATUS_SUCCESS {
            Ok(())
        } else {
            Err(ntstatus_error(
                "NtSetSystemInformation(SystemMemoryListInformation)",
                status,
            ))
        }
    }

    fn enable_privilege(
        privilege: &str,
        context: &'static str,
    ) -> std::result::Result<(), WinApiError> {
        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            ) == 0
            {
                return Err(last_error("OpenProcessToken"));
            }
            let token = Handle(token);

            let mut luid: LUID = zeroed();
            let privilege_name = wide_null(privilege);
            if LookupPrivilegeValueW(std::ptr::null(), privilege_name.as_ptr(), &mut luid) == 0 {
                return Err(last_error("LookupPrivilegeValueW"));
            }

            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };

            if AdjustTokenPrivileges(
                token.0,
                0,
                &privileges,
                size_of::<TOKEN_PRIVILEGES>() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(last_error("AdjustTokenPrivileges"));
            }

            let code = GetLastError();
            if code == ERROR_NOT_ALL_ASSIGNED {
                return Err(WinApiError {
                    context,
                    code,
                    message:
                        "current token does not hold the required privilege; run as administrator"
                            .to_owned(),
                });
            }
        }
        Ok(())
    }

    fn memory_list_command_value(command: MemoryListCommand) -> i32 {
        match command {
            MemoryListCommand::EmptyWorkingSets => 2,
            MemoryListCommand::FlushModifiedList => 3,
            MemoryListCommand::PurgeStandbyList => 4,
            MemoryListCommand::PurgeLowPriorityStandbyList => 5,
        }
    }

    fn open_process(
        pid: u32,
        desired_access: u32,
        context: &'static str,
    ) -> std::result::Result<Handle, WinApiError> {
        unsafe {
            let handle = Handle(OpenProcess(desired_access, 0, pid));
            if handle.is_invalid() {
                Err(last_error(context))
            } else {
                Ok(handle)
            }
        }
    }

    fn open_process_for_query(pid: u32) -> std::result::Result<Handle, WinApiError> {
        open_process(
            pid,
            PROCESS_QUERY_LIMITED_INFORMATION,
            "OpenProcess(query limited)",
        )
        .or_else(|_| open_process(pid, PROCESS_QUERY_INFORMATION, "OpenProcess(query)"))
    }

    fn process_memory_from_handle(
        handle: HANDLE,
    ) -> std::result::Result<(u64, Option<u64>), WinApiError> {
        unsafe {
            let mut counters: PROCESS_MEMORY_COUNTERS_EX = zeroed();
            counters.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
            let counters_ptr =
                &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS;
            if K32GetProcessMemoryInfo(
                handle,
                counters_ptr,
                size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ) == 0
            {
                return Err(last_error("K32GetProcessMemoryInfo"));
            }
            Ok((
                counters.WorkingSetSize as u64,
                Some(counters.PrivateUsage as u64),
            ))
        }
    }

    fn process_image_path(handle: HANDLE) -> std::result::Result<String, WinApiError> {
        unsafe {
            let mut buffer = vec![0u16; 32768];
            let len = K32GetProcessImageFileNameW(handle, buffer.as_mut_ptr(), buffer.len() as u32);
            if len == 0 {
                return Err(last_error("K32GetProcessImageFileNameW"));
            }
            buffer.truncate(len as usize);
            Ok(String::from_utf16_lossy(&buffer))
        }
    }

    fn wide_array_to_string(value: &[u16]) -> String {
        let len = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
        String::from_utf16_lossy(&value[..len])
    }

    fn wide_null(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn last_error(context: &'static str) -> WinApiError {
        let code = unsafe { GetLastError() };
        let message = std::io::Error::from_raw_os_error(code as i32).to_string();
        WinApiError {
            context,
            code,
            message,
        }
    }

    fn ntstatus_error(context: &'static str, status: i32) -> WinApiError {
        let message = match status {
            STATUS_PRIVILEGE_NOT_HELD => {
                "privilege not held; run MemSpark as administrator".to_owned()
            }
            STATUS_ACCESS_DENIED => "access denied; run MemSpark as administrator".to_owned(),
            _ => format!("NTSTATUS 0x{:08X}", status as u32),
        };
        WinApiError {
            context,
            code: status as u32,
            message,
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{MemoryListCommand, ProcessQueryInfo, RawProcessInfo, WinApiError};
    use crate::{memory::MemorySnapshot, MemSparkError, Result};

    pub fn current_process_id() -> u32 {
        std::process::id()
    }

    pub fn foreground_process_id() -> Result<Option<u32>> {
        Ok(None)
    }

    pub fn memory_snapshot() -> Result<MemorySnapshot> {
        Err(MemSparkError::Unsupported(
            "MemSpark core memory snapshot requires Windows".to_owned(),
        ))
    }

    pub fn enumerate_processes_raw() -> Result<Vec<RawProcessInfo>> {
        Err(MemSparkError::Unsupported(
            "MemSpark process enumeration requires Windows".to_owned(),
        ))
    }

    pub fn query_process(_pid: u32) -> std::result::Result<ProcessQueryInfo, WinApiError> {
        Err(unsupported("query_process"))
    }

    pub fn query_process_working_set(_pid: u32) -> std::result::Result<u64, WinApiError> {
        Err(unsupported("query_process_working_set"))
    }

    pub fn trim_process_empty_working_set(_pid: u32) -> std::result::Result<(), WinApiError> {
        Err(unsupported("trim_process_empty_working_set"))
    }

    pub fn trim_process_set_working_set_size_ex(_pid: u32) -> std::result::Result<(), WinApiError> {
        Err(unsupported("trim_process_set_working_set_size_ex"))
    }

    pub fn graceful_close_process_windows(_pid: u32) -> std::result::Result<u32, WinApiError> {
        Err(unsupported("graceful_close_process_windows"))
    }

    pub fn wait_for_process_exit(
        _pid: u32,
        _timeout_ms: u32,
    ) -> std::result::Result<bool, WinApiError> {
        Err(unsupported("wait_for_process_exit"))
    }

    pub fn terminate_process(_pid: u32) -> std::result::Result<(), WinApiError> {
        Err(unsupported("terminate_process"))
    }

    pub fn is_running_as_admin() -> bool {
        false
    }

    pub fn enable_profile_privilege() -> std::result::Result<(), WinApiError> {
        Err(unsupported("enable_profile_privilege"))
    }

    pub fn enable_debug_privilege() -> std::result::Result<(), WinApiError> {
        Err(unsupported("enable_debug_privilege"))
    }

    pub fn enable_increase_quota_privilege() -> std::result::Result<(), WinApiError> {
        Err(unsupported("enable_increase_quota_privilege"))
    }

    pub fn run_elevated_and_wait(
        _executable: &str,
        _parameters: &str,
    ) -> std::result::Result<u32, WinApiError> {
        Err(unsupported("run_elevated_and_wait"))
    }

    pub fn set_system_memory_list(
        _command: MemoryListCommand,
    ) -> std::result::Result<(), WinApiError> {
        Err(unsupported("set_system_memory_list"))
    }

    fn unsupported(context: &'static str) -> WinApiError {
        WinApiError {
            context,
            code: 0,
            message: "Windows-only API".to_owned(),
        }
    }
}

pub use imp::{
    current_process_id, enable_debug_privilege, enable_increase_quota_privilege,
    enable_profile_privilege, enumerate_processes_raw, foreground_process_id,
    graceful_close_process_windows, is_running_as_admin, memory_snapshot, query_process,
    query_process_working_set, run_elevated_and_wait, set_system_memory_list, terminate_process,
    trim_process_empty_working_set, trim_process_set_working_set_size_ex, wait_for_process_exit,
};
