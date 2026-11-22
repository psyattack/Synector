use crate::models::ProcessSnapshotItem;
use std::collections::HashMap;

#[cfg(windows)]
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
#[cfg(windows)]
use windows::core::BOOL;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
#[cfg(windows)]
use windows::Win32::System::Diagnostics::Etw::{
    CloseTrace, ControlTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace, StartTraceW,
    CONTROLTRACE_HANDLE, EVENT_RECORD, EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_LOGFILEW,
    EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE, PROCESSTRACE_HANDLE,
    PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_REAL_TIME, WNODE_FLAG_TRACED_GUID,
};
#[cfg(windows)]
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
#[cfg(windows)]
use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
#[cfg(windows)]
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEvent {
    pub process_name: String,
    pub pid: u32,
    pub is_started: bool,
}

#[cfg(windows)]
#[repr(C)]
struct EtwPropertiesBuffer {
    props: EVENT_TRACE_PROPERTIES,
    session_name: [u16; 128],
    log_file_name: [u16; 128],
}

#[cfg(windows)]
impl EtwPropertiesBuffer {
    fn new(name: &[u16]) -> Self {
        let mut buf: Self = unsafe { std::mem::zeroed() };
        buf.props.Wnode.BufferSize = std::mem::size_of::<Self>() as u32;
        buf.props.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        buf.props.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        buf.props.LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
        buf.props.LogFileNameOffset = 0;

        let copy_len = name.len().min(buf.session_name.len() - 1);
        buf.session_name[..copy_len].copy_from_slice(&name[..copy_len]);
        buf
    }
}

#[cfg(windows)]
const KERNEL_PROCESS_GUID: windows::core::GUID = windows::core::GUID::from_values(
    0x22fb2cd6,
    0x0e7b,
    0x422b,
    [0xa0, 0xc7, 0x2f, 0xad, 0x1f, 0xd0, 0xe7, 0x16],
);

/// Raw event queued by the realtime ETW callback and resolved later on the daemon thread.
/// The callback must stay brief (ETW guidance), so it carries no process name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RawProcessEvent {
    pid: u32,
    is_started: bool,
}

#[cfg(windows)]
const ETW_EVENT_QUEUE_CAPACITY: usize = 2000;

#[cfg(windows)]
static ETW_EVENT_QUEUE: std::sync::Mutex<Vec<RawProcessEvent>> = std::sync::Mutex::new(Vec::new());
#[cfg(windows)]
static ETW_DROPPED_EVENTS: AtomicU32 = AtomicU32::new(0);
/// Set by the trace thread when `ProcessTrace` returns, so the watcher can fall back to polling.
#[cfg(windows)]
static ETW_TRACE_STOPPED: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static ETW_TRACE_EXIT_CODE: AtomicU32 = AtomicU32::new(0);

#[cfg(windows)]
unsafe extern "system" fn etw_event_record_callback(record: *mut EVENT_RECORD) {
    if record.is_null() {
        return;
    }
    let rec = &*record;
    if rec.EventHeader.ProviderId != KERNEL_PROCESS_GUID {
        return;
    }
    let id = rec.EventHeader.EventDescriptor.Id;
    if (id != 1 && id != 2) || rec.UserDataLength < 4 || rec.UserData.is_null() {
        return;
    }
    let pid = *(rec.UserData as *const u32);
    if pid == 0 {
        return;
    }

    if let Ok(mut q) = ETW_EVENT_QUEUE.lock() {
        if q.len() < ETW_EVENT_QUEUE_CAPACITY {
            q.push(RawProcessEvent {
                pid,
                is_started: id == 1,
            });
        } else {
            ETW_DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(windows)]
struct EtwSession {
    session_handle: CONTROLTRACE_HANDLE,
    trace_handle: PROCESSTRACE_HANDLE,
    session_name_wide: Vec<u16>,
}

/// The ETW session name is machine-wide while daemons are per Windows session, so the name
/// carries the session id: a second user's daemon must never stop this session to start its own.
#[cfg(windows)]
fn etw_session_name() -> Vec<u16> {
    use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows::Win32::System::Threading::GetCurrentProcessId;

    let mut session_id = 0u32;
    let name = unsafe {
        match ProcessIdToSessionId(GetCurrentProcessId(), &mut session_id) {
            Ok(()) => format!("SynectorKernelProcessTrace-{session_id}"),
            Err(_) => "SynectorKernelProcessTrace".to_string(),
        }
    };
    name.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
impl EtwSession {
    fn try_start() -> Option<Self> {
        use windows::core::PCWSTR;

        let session_name_wide: Vec<u16> = etw_session_name();

        unsafe {
            let mut session_handle = CONTROLTRACE_HANDLE::default();
            let mut buf = EtwPropertiesBuffer::new(&session_name_wide);

            let mut err = StartTraceW(
                &mut session_handle,
                PCWSTR(session_name_wide.as_ptr()),
                &mut buf.props,
            );
            if err.0 == 183 {
                // ERROR_ALREADY_EXISTS: close the stale session left by an earlier run of this
                // same Windows session and retry.
                let mut stop_buf = EtwPropertiesBuffer::new(&session_name_wide);
                let _ = ControlTraceW(
                    CONTROLTRACE_HANDLE::default(),
                    PCWSTR(session_name_wide.as_ptr()),
                    &mut stop_buf.props,
                    EVENT_TRACE_CONTROL_STOP,
                );
                err = StartTraceW(
                    &mut session_handle,
                    PCWSTR(session_name_wide.as_ptr()),
                    &mut buf.props,
                );
            }

            if err.0 != 0 {
                return None;
            }

            let enable_err = EnableTraceEx2(
                session_handle,
                &KERNEL_PROCESS_GUID,
                1,                  // EVENT_CONTROL_CODE_ENABLE_PROVIDER
                4,                  // TRACE_LEVEL_INFORMATION
                0x8000000000000010, // Process keyword
                0,
                0,
                None,
            );
            if enable_err.0 != 0 {
                let mut stop_buf = EtwPropertiesBuffer::new(&session_name_wide);
                let _ = ControlTraceW(
                    session_handle,
                    PCWSTR(session_name_wide.as_ptr()),
                    &mut stop_buf.props,
                    EVENT_TRACE_CONTROL_STOP,
                );
                return None;
            }

            let mut logfile = EVENT_TRACE_LOGFILEW::default();
            let mut logger_name_wide = session_name_wide.clone();
            logfile.LoggerName = windows::core::PWSTR(logger_name_wide.as_mut_ptr());
            logfile.Anonymous1.ProcessTraceMode =
                PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
            logfile.Anonymous2.EventRecordCallback = Some(etw_event_record_callback);

            let trace_handle = OpenTraceW(&mut logfile);
            if trace_handle.Value == 0 || trace_handle.Value == u64::MAX {
                let mut stop_buf = EtwPropertiesBuffer::new(&session_name_wide);
                let _ = ControlTraceW(
                    session_handle,
                    PCWSTR(session_name_wide.as_ptr()),
                    &mut stop_buf.props,
                    EVENT_TRACE_CONTROL_STOP,
                );
                return None;
            }

            let th = trace_handle;
            ETW_TRACE_EXIT_CODE.store(0, Ordering::SeqCst);
            ETW_TRACE_STOPPED.store(false, Ordering::SeqCst);
            ETW_DROPPED_EVENTS.store(0, Ordering::Relaxed);
            if std::thread::Builder::new()
                .name("etw-kernel-process-trace".into())
                .spawn(move || {
                    let result = ProcessTrace(&[th], None, None);
                    ETW_TRACE_EXIT_CODE.store(result.0, Ordering::SeqCst);
                    ETW_TRACE_STOPPED.store(true, Ordering::SeqCst);
                })
                .is_err()
            {
                let _ = CloseTrace(trace_handle);
                let mut stop_buf = EtwPropertiesBuffer::new(&session_name_wide);
                let _ = ControlTraceW(
                    session_handle,
                    PCWSTR(session_name_wide.as_ptr()),
                    &mut stop_buf.props,
                    EVENT_TRACE_CONTROL_STOP,
                );
                return None;
            }

            Some(Self {
                session_handle,
                trace_handle,
                session_name_wide,
            })
        }
    }
}

#[cfg(windows)]
impl Drop for EtwSession {
    fn drop(&mut self) {
        use windows::core::PCWSTR;
        unsafe {
            if self.trace_handle.Value != 0 && self.trace_handle.Value != u64::MAX {
                let _ = CloseTrace(self.trace_handle);
            }
            if self.session_handle.Value != 0 {
                let mut buf = EtwPropertiesBuffer::new(&self.session_name_wide);
                let _ = ControlTraceW(
                    self.session_handle,
                    PCWSTR(self.session_name_wide.as_ptr()),
                    &mut buf.props,
                    EVENT_TRACE_CONTROL_STOP,
                );
            }
        }
    }
}

pub struct ProcessWatcher {
    known_processes: HashMap<u32, String>,
    #[cfg(windows)]
    etw_session: Option<EtwSession>,
    last_reconcile: std::time::Instant,
}

impl ProcessWatcher {
    pub fn new() -> Self {
        let initial = Self::snapshot_process_map();
        #[cfg(windows)]
        let etw = if is_elevated() {
            match EtwSession::try_start() {
                Some(session) => {
                    crate::logging::info(
                        "WATCHER",
                        "Kernel ETW real-time process tracing is active",
                    );
                    Some(session)
                }
                None => {
                    crate::logging::warn(
                        "WATCHER",
                        "Kernel ETW tracing could not start; falling back to Toolhelp32 snapshot polling",
                    );
                    None
                }
            }
        } else {
            crate::logging::info(
                "WATCHER",
                "Process tracing is not elevated; using Toolhelp32 snapshot polling",
            );
            None
        };

        Self {
            known_processes: initial,
            #[cfg(windows)]
            etw_session: etw,
            last_reconcile: std::time::Instant::now(),
        }
    }

    pub fn is_etw_active(&self) -> bool {
        #[cfg(windows)]
        {
            etw_session_active(
                self.etw_session.is_some(),
                ETW_TRACE_STOPPED.load(Ordering::SeqCst),
            )
        }
    }

    pub fn get_mode_name(&self) -> &'static str {
        if self.is_etw_active() {
            "Kernel ETW (Real-Time Push)"
        } else {
            "Adaptive Snapshot Polling"
        }
    }

    pub fn poll_changes(&mut self) -> Vec<ProcessEvent> {
        #[cfg(windows)]
        if self.etw_session.is_some() {
            if ETW_TRACE_STOPPED.load(Ordering::SeqCst) {
                let code = ETW_TRACE_EXIT_CODE.load(Ordering::SeqCst);
                crate::logging::warn(
                    "WATCHER",
                    &format!(
                        "Kernel ETW trace stopped (code {code}); falling back to Toolhelp32 snapshot polling"
                    ),
                );
                // Dropping the session releases the trace and control handles; the adaptive
                // polling path below then takes over within this same tick.
                self.etw_session = None;
            } else {
                return self.poll_etw_changes();
            }
        }

        self.poll_toolhelp_changes()
    }

    #[cfg(windows)]
    fn poll_etw_changes(&mut self) -> Vec<ProcessEvent> {
        let events = ETW_EVENT_QUEUE
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default();

        let dropped = ETW_DROPPED_EVENTS.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            crate::logging::warn(
                "WATCHER",
                &format!("Kernel ETW event queue overflowed; {dropped} process events were dropped"),
            );
        }

        let mut out = apply_etw_events(
            events,
            &mut self.known_processes,
            Self::get_process_image_path,
            Self::snapshot_process_map,
        );

        // Periodic background reconciliation every 10 seconds to guarantee no drift
        if self.last_reconcile.elapsed().as_secs() >= 10 {
            self.last_reconcile = std::time::Instant::now();
            let current = Self::snapshot_process_map();
            out.extend(reconcile_known_processes(&mut self.known_processes, &current));
        }

        out
    }

    fn poll_toolhelp_changes(&mut self) -> Vec<ProcessEvent> {
        let current = Self::snapshot_process_map();
        let events = diff_process_maps(&self.known_processes, &current);
        self.known_processes = current;
        events
    }

    #[cfg(windows)]
    pub fn snapshot_process_map() -> HashMap<u32, String> {
        let mut map = HashMap::new();
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot.is_err() {
                return map;
            }
            let handle = snapshot.unwrap();

            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(handle, &mut entry).is_ok() {
                loop {
                    let name_len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let name =
                        String::from_utf16_lossy(&entry.szExeFile[..name_len]).to_lowercase();
                    map.insert(entry.th32ProcessID, name);

                    if Process32NextW(handle, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(handle);
        }
        map
    }

    #[cfg(windows)]
    pub fn get_process_image_path(pid: u32) -> Option<String> {
        unsafe {
            if let Ok(h_proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                let mut buf = [0u16; 1024];
                let mut size = buf.len() as u32;
                let res = QueryFullProcessImageNameW(
                    h_proc,
                    PROCESS_NAME_FORMAT(0),
                    windows::core::PWSTR(buf.as_mut_ptr()),
                    &mut size,
                );
                let _ = CloseHandle(h_proc);
                if res.is_ok() {
                    return Some(String::from_utf16_lossy(&buf[..size as usize]));
                }
            }
            None
        }
    }

    pub fn find_exe_path_by_name(process_name: &str) -> Option<String> {
        let lower = process_name.to_lowercase();
        let stripped_lower = lower.strip_suffix(".exe").unwrap_or(&lower);
        let map = Self::snapshot_process_map();
        for (&pid, name) in &map {
            let name_lower = name.to_lowercase();
            let name_stripped = name_lower.strip_suffix(".exe").unwrap_or(&name_lower);
            if name_lower == lower || name_stripped == stripped_lower {
                if let Some(path) = Self::get_process_image_path(pid) {
                    return Some(path);
                }
            }
        }
        let sys_root = std::env::var_os("SystemRoot")
            .or_else(|| std::env::var_os("WINDIR"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("C:\\Windows"));
        let exe_name = if lower.ends_with(".exe") {
            process_name.to_string()
        } else {
            format!("{}.exe", process_name)
        };
        let sys32_candidate = sys_root.join("System32").join(&exe_name);
        if sys32_candidate.exists() {
            return Some(sys32_candidate.to_string_lossy().to_string());
        }
        None
    }

    #[cfg(windows)]
    pub fn get_detailed_snapshot() -> Vec<ProcessSnapshotItem> {
        let mut items = Vec::new();
        let window_titles = Self::get_window_titles();

        unsafe {
            let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
                Ok(h) => h,
                Err(_) => return items,
            };

            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    let pid = entry.th32ProcessID;
                    if pid != 0 {
                        let name_len = entry
                            .szExeFile
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(entry.szExeFile.len());
                        let name = String::from_utf16_lossy(&entry.szExeFile[..name_len]);

                        let mut mem_bytes = 0u64;
                        if let Ok(h_proc) =
                            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
                        {
                            let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
                            pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
                            if K32GetProcessMemoryInfo(h_proc, &mut pmc, pmc.cb).as_bool() {
                                mem_bytes = pmc.WorkingSetSize as u64;
                            }
                            let _ = CloseHandle(h_proc);
                        }

                        let window_title = window_titles.get(&pid).cloned().unwrap_or_default();

                        items.push(ProcessSnapshotItem {
                            pid,
                            name,
                            window_title,
                            memory_bytes: mem_bytes,
                            cpu_percent: 0.0,
                        });
                    }

                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }

        // Sort by memory descending
        items.sort_by_key(|a| std::cmp::Reverse(a.memory_bytes));
        items
    }

    #[cfg(windows)]
    fn get_window_titles() -> HashMap<u32, String> {
        let mut map: HashMap<u32, String> = HashMap::new();
        unsafe {
            unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
                let map = &mut *(lparam.0 as *mut HashMap<u32, String>);
                if IsWindowVisible(hwnd).as_bool() {
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(hwnd, Some(&mut pid));
                    if pid != 0 && !map.contains_key(&pid) {
                        let mut buf = [0u16; 256];
                        let len = GetWindowTextW(hwnd, &mut buf);
                        if len > 0 {
                            let title = String::from_utf16_lossy(&buf[..len as usize]);
                            if !title.trim().is_empty() {
                                map.insert(pid, title);
                            }
                        }
                    }
                }
                BOOL(1)
            }

            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut map as *mut _ as isize));
        }
        map
    }
}

/// Lowercased executable file name of an image path.
fn process_name_from_path(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .map(|s| s.to_lowercase())
        .filter(|s| !s.is_empty())
}

/// Turns raw ETW events into the shared process-event surface. The name of a started process is
/// resolved from the image path first and from a Toolhelp snapshot second, so protected/PPL
/// processes whose image path cannot be opened still produce the same start event the polling
/// path produces, and their stop event is not lost. The pid is always recorded.
fn apply_etw_events(
    events: Vec<RawProcessEvent>,
    known: &mut HashMap<u32, String>,
    mut image_path: impl FnMut(u32) -> Option<String>,
    mut snapshot: impl FnMut() -> HashMap<u32, String>,
) -> Vec<ProcessEvent> {
    let mut out = Vec::with_capacity(events.len());
    let mut snapshot_map: Option<HashMap<u32, String>> = None;

    for ev in events {
        if ev.is_started {
            let mut name = image_path(ev.pid)
                .as_deref()
                .and_then(process_name_from_path);
            if name.is_none() {
                let map = snapshot_map.get_or_insert_with(&mut snapshot);
                name = map.get(&ev.pid).cloned();
            }
            let name = name.unwrap_or_default();
            known.insert(ev.pid, name.clone());
            if !name.is_empty() {
                out.push(ProcessEvent {
                    process_name: name,
                    pid: ev.pid,
                    is_started: true,
                });
            }
        } else if let Some(name) = known.remove(&ev.pid) {
            if !name.is_empty() {
                out.push(ProcessEvent {
                    process_name: name,
                    pid: ev.pid,
                    is_started: false,
                });
            }
        }
    }

    out
}

/// Reconciles the ETW path's known set with a fresh Toolhelp snapshot.
///
/// A start whose image path and Toolhelp fallback both failed is recorded with an empty name and
/// emits nothing, so the periodic reconcile retries the lookup; when the snapshot resolves the
/// pid, this emits the start event the polling path would have produced and both paths keep
/// feeding the same trigger surface. A pid that already exited cannot be named any more: it is
/// dropped with a debug line (polling would have retried on its next tick, so this is the only
/// remaining case where the two paths can still differ).
fn reconcile_known_processes(
    known: &mut HashMap<u32, String>,
    current: &HashMap<u32, String>,
) -> Vec<ProcessEvent> {
    let mut out = Vec::new();

    for (&pid, name) in current {
        match known.get_mut(&pid) {
            Some(known_name) if known_name.is_empty() => {
                known_name.clone_from(name);
                out.push(ProcessEvent {
                    process_name: name.clone(),
                    pid,
                    is_started: true,
                });
            }
            Some(_) => {}
            None => {
                // Drift correction only: record a pid the ETW queue never reported so its exit
                // is paired. The polling path emits no event for a process it already knew.
                known.insert(pid, name.clone());
            }
        }
    }

    let mut dropped_unresolved = Vec::new();
    known.retain(|&pid, known_name| {
        if !current.contains_key(&pid) && known_name.is_empty() {
            dropped_unresolved.push(pid);
        }
        current.contains_key(&pid)
    });
    for pid in dropped_unresolved {
        crate::logging::debug(
            "WATCHER",
            &format!(
                "ETW known process {pid} exited before its name could be resolved; no process event was emitted for it"
            ),
        );
    }

    out
}

/// Snapshot differential used by the adaptive Toolhelp32 polling path.
fn diff_process_maps(
    before: &HashMap<u32, String>,
    after: &HashMap<u32, String>,
) -> Vec<ProcessEvent> {
    let mut events = Vec::new();

    for (&pid, name) in after {
        if !before.contains_key(&pid) {
            events.push(ProcessEvent {
                process_name: name.clone(),
                pid,
                is_started: true,
            });
        }
    }

    for (&pid, name) in before {
        if !after.contains_key(&pid) {
            events.push(ProcessEvent {
                process_name: name.clone(),
                pid,
                is_started: false,
            });
        }
    }

    events
}

#[cfg(windows)]
fn etw_session_active(has_session: bool, trace_stopped: bool) -> bool {
    has_session && !trace_stopped
}

#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_ok() {
            let mut elevation = TOKEN_ELEVATION::default();
            let mut ret_len = 0u32;
            let success = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut ret_len,
            )
            .is_ok();
            let _ = CloseHandle(token);
            if success {
                return elevation.TokenIsElevated != 0;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_etw_imports() {
        #[cfg(windows)]
        {
            use windows::Win32::System::Diagnostics::Etw::*;
            let _ = EVENT_TRACE_CONTROL_STOP;
            let _ = EVENT_TRACE_REAL_TIME_MODE;
            let _ = PROCESS_TRACE_MODE_REAL_TIME;
            let _ = PROCESS_TRACE_MODE_EVENT_RECORD;
            let _ = WNODE_FLAG_TRACED_GUID;

            let logfile = EVENT_TRACE_LOGFILEW::default();
            let _ = logfile.LoggerName;

            // Ensure is_elevated() query executes without panicking regardless of token elevation state
            let _ = crate::daemon::watcher::is_elevated();
        }
    }

    #[test]
    fn test_process_watcher_initialization_and_mode() {
        let mut watcher = ProcessWatcher::new();
        assert!(!watcher.known_processes.is_empty());
        let mode = watcher.get_mode_name();
        assert!(mode.contains("ETW") || mode.contains("Snapshot"));
        let _ = watcher.poll_changes();
    }

    #[test]
    fn test_etw_and_toolhelp_paths_agree_on_unresolved_start() {
        // A protected/PPL process: OpenProcess fails, but a Toolhelp snapshot still lists it.
        let before: HashMap<u32, String> = HashMap::new();
        let after: HashMap<u32, String> =
            [(4321u32, "protected.exe".to_string())].into_iter().collect();

        let mut known = before.clone();
        let etw = apply_etw_events(
            vec![RawProcessEvent {
                pid: 4321,
                is_started: true,
            }],
            &mut known,
            |_| None,
            || after.clone(),
        );
        let toolhelp = diff_process_maps(&before, &after);
        assert_eq!(
            etw, toolhelp,
            "ETW and Toolhelp paths must emit the same start event"
        );
        assert_eq!(known.get(&4321).map(String::as_str), Some("protected.exe"));

        // The matching stop event must not be dropped just because the image path was unreadable.
        let etw_stop = apply_etw_events(
            vec![RawProcessEvent {
                pid: 4321,
                is_started: false,
            }],
            &mut known,
            |_| None,
            HashMap::new,
        );
        let toolhelp_stop = diff_process_maps(&after, &HashMap::new());
        assert_eq!(
            etw_stop, toolhelp_stop,
            "ETW and Toolhelp paths must emit the same stop event"
        );
        assert!(!known.contains_key(&4321));
    }

    #[test]
    fn test_etw_reconcile_fills_a_name_missed_at_start() {
        // Both the image path and the Toolhelp fallback failed when the start was seen, so the
        // pid is known with an empty name and no start event was ever emitted.
        let mut known: HashMap<u32, String> = [
            (4321u32, String::new()),
            (1234u32, "known.exe".to_string()),
        ]
        .into_iter()
        .collect();
        let current: HashMap<u32, String> = [
            (4321u32, "protected.exe".to_string()),
            (1234u32, "known.exe".to_string()),
        ]
        .into_iter()
        .collect();

        let events = reconcile_known_processes(&mut known, &current);
        assert_eq!(
            events,
            vec![ProcessEvent {
                process_name: "protected.exe".to_string(),
                pid: 4321,
                is_started: true,
            }],
            "the reconcile must emit only the start event the polling path would have produced"
        );
        assert_eq!(known.get(&4321).map(String::as_str), Some("protected.exe"));

        // The repaired entry now pairs its stop event like any other known process.
        let stop = apply_etw_events(
            vec![RawProcessEvent {
                pid: 4321,
                is_started: false,
            }],
            &mut known,
            |_| None,
            HashMap::new,
        );
        assert_eq!(
            stop,
            vec![ProcessEvent {
                process_name: "protected.exe".to_string(),
                pid: 4321,
                is_started: false,
            }]
        );
    }

    #[test]
    fn test_etw_reconcile_keeps_unresolved_pids_silent_when_they_are_gone() {
        let mut known: HashMap<u32, String> = [(7u32, String::new())].into_iter().collect();
        let events = reconcile_known_processes(&mut known, &HashMap::new());
        assert!(
            events.is_empty(),
            "a process that exited before it could be named has no event to emit"
        );
        assert!(!known.contains_key(&7), "the stale pid must still be reconciled away");
    }

    #[test]
    fn test_etw_start_prefers_image_path_name() {
        let mut known = HashMap::new();
        let events = apply_etw_events(
            vec![RawProcessEvent {
                pid: 77,
                is_started: true,
            }],
            &mut known,
            |_| Some("C:\\Windows\\System32\\Notepad.EXE".to_string()),
            HashMap::new,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].process_name, "notepad.exe");
        assert_eq!(known.get(&77).map(String::as_str), Some("notepad.exe"));
    }

    #[test]
    #[cfg(windows)]
    fn test_etw_active_tracks_trace_exit() {
        assert!(etw_session_active(true, false));
        assert!(!etw_session_active(true, true));
        assert!(!etw_session_active(false, false));
    }

    #[test]
    #[cfg(windows)]
    fn test_etw_session_name_is_wide_and_scoped() {
        let name = etw_session_name();
        assert!(name.len() > 1);
        assert_eq!(name.last(), Some(&0), "the session name must be NUL-terminated");
        let text = String::from_utf16_lossy(&name[..name.len() - 1]);
        assert!(text.starts_with("SynectorKernelProcessTrace"));
    }
}
