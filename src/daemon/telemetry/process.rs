use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(windows)]
use super::system::filetime_to_u64;

#[cfg(windows)]
use windows::Win32::Foundation::FILETIME;

#[cfg(windows)]
pub(super) fn get_target_process_metrics(target_name: &str) -> (Option<u64>, Option<f32>) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

    let target_clean = std::path::Path::new(target_name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(target_name)
        .to_lowercase();

    unsafe {
        let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return (None, None),
        };

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        let mut found_pid = None;
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let name = String::from_utf16_lossy(&entry.szExeFile)
                    .trim_matches('\0')
                    .to_lowercase();
                if name == target_clean {
                    found_pid = Some(entry.th32ProcessID);
                    break;
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);

        if let Some(pid) = found_pid {
            if let Ok(h_proc) = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid) {
                let mut pmc = PROCESS_MEMORY_COUNTERS {
                    cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                    ..Default::default()
                };
                let ram_mb = if GetProcessMemoryInfo(h_proc, &mut pmc, pmc.cb).is_ok() {
                    Some((pmc.WorkingSetSize / (1024 * 1024)) as u64)
                } else {
                    None
                };
                let cpu_pct = sample_process_cpu(pid, h_proc);
                let _ = CloseHandle(h_proc);
                return (ram_mb, cpu_pct);
            }
        }
    }
    (None, None)
}

struct ProcessCpuSampler {
    pid: u32,
    sampled_at: Instant,
    total_100ns: u64,
}

static PROCESS_CPU_SAMPLER: Mutex<Option<ProcessCpuSampler>> = Mutex::new(None);

/// Process CPU is a delta of kernel+user `GetProcessTimes` against wall-clock time, normalised to
/// all logical cores so the value matches the 0-100 % figure shown by Task Manager. The first
/// sample after a process appears has no interval yet and is reported as unknown.
#[cfg(windows)]
fn sample_process_cpu(pid: u32, process: windows::Win32::Foundation::HANDLE) -> Option<f32> {
    use windows::Win32::System::Threading::GetProcessTimes;

    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    if unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }.is_err()
    {
        return None;
    }
    let total_100ns = filetime_to_u64(kernel).saturating_add(filetime_to_u64(user));

    let now = Instant::now();
    let mut lock = PROCESS_CPU_SAMPLER.lock().ok()?;
    let percent = match lock.as_ref() {
        Some(state) if state.pid == pid => process_cpu_percent(
            total_100ns.saturating_sub(state.total_100ns),
            now.duration_since(state.sampled_at),
            cpu_core_count(),
        ),
        _ => None,
    };
    *lock = Some(ProcessCpuSampler {
        pid,
        sampled_at: now,
        total_100ns,
    });
    percent
}

pub(super) fn process_cpu_percent(delta_100ns: u64, elapsed: Duration, cores: u32) -> Option<f32> {
    let elapsed_100ns = elapsed.as_secs_f64() * 10_000_000.0;
    if elapsed_100ns <= 0.0 || cores == 0 {
        return None;
    }
    Some(((delta_100ns as f64 / elapsed_100ns) * 100.0 / cores as f64).clamp(0.0, 100.0) as f32)
}

fn cpu_core_count() -> u32 {
    std::thread::available_parallelism()
        .map(|cores| cores.get() as u32)
        .unwrap_or(1)
}
