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

/// Previous sample of the daemon's own counters, so the next one can be reported as a rate.
#[cfg(windows)]
struct DaemonUsageSampler {
    sampled_at: Instant,
    cpu_100ns: u64,
    io_read: u64,
    io_write: u64,
}

#[cfg(windows)]
static DAEMON_USAGE: Mutex<Option<DaemonUsageSampler>> = Mutex::new(None);

/// Private working set of this process: the memory figure Task Manager reports, not
/// `WorkingSetSize`, which also counts shared DLL pages.
pub fn daemon_memory_bytes() -> u64 {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::ProcessStatus::{
            K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
        };
        use windows::Win32::System::Threading::GetCurrentProcess;
        let mut pmc: PROCESS_MEMORY_COUNTERS_EX2 = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
        let counters = (&mut pmc as *mut PROCESS_MEMORY_COUNTERS_EX2).cast::<PROCESS_MEMORY_COUNTERS>();
        if K32GetProcessMemoryInfo(GetCurrentProcess(), counters, pmc.cb).as_bool() {
            if pmc.PrivateWorkingSetSize > 0 {
                return pmc.PrivateWorkingSetSize as u64;
            }
            if pmc.PrivateUsage > 0 {
                return pmc.PrivateUsage as u64;
            }
            return pmc.WorkingSetSize as u64;
        }
        // Older builds may not answer the EX2 size; the base structure still reports the working
        // set, which is better than the fallback constant.
        let mut base: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        base.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut base, base.cb).as_bool() {
            return base.WorkingSetSize as u64;
        }
    }
    2 * 1024 * 1024 // ~2MB default
}

/// One sample of the daemon's own resource use: CPU over the interval since the previous sample,
/// the private working set and the disk throughput derived from the process I/O counters. The
/// first call only records the baseline and reports no CPU percentage and no disk rate.
pub fn sample_daemon_usage() -> crate::models::DaemonUsage {
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{
            GetCurrentProcess, GetProcessIoCounters, GetProcessTimes, IO_COUNTERS,
        };

        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let mut io = IO_COUNTERS::default();
        let process = unsafe { GetCurrentProcess() };
        let times_ok = unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }.is_ok();
        let io_ok = unsafe { GetProcessIoCounters(process, &mut io) }.is_ok();

        let cpu_100ns = filetime_to_u64(kernel).saturating_add(filetime_to_u64(user));
        let io_read = io.ReadTransferCount;
        let io_write = io.WriteTransferCount;
        let now = Instant::now();

        let mut lock = match DAEMON_USAGE.lock() {
            Ok(lock) => lock,
            Err(_) => {
                return crate::models::DaemonUsage {
                    ram_bytes: daemon_memory_bytes(),
                    ..Default::default()
                };
            }
        };
        let usage = match lock.as_ref() {
            Some(previous) if times_ok && io_ok => {
                let elapsed = now.duration_since(previous.sampled_at);
                let seconds = elapsed.as_secs_f64();
                let per_second = |total: u64, previous: u64| -> u64 {
                    if seconds <= 0.0 {
                        return 0;
                    }
                    ((total.saturating_sub(previous)) as f64 / seconds).round() as u64
                };
                crate::models::DaemonUsage {
                    cpu_pct: process_cpu_percent(
                        cpu_100ns.saturating_sub(previous.cpu_100ns),
                        elapsed,
                        cpu_core_count(),
                    ),
                    ram_bytes: daemon_memory_bytes(),
                    disk_read_bps: per_second(io_read, previous.io_read),
                    disk_write_bps: per_second(io_write, previous.io_write),
                }
            }
            _ => crate::models::DaemonUsage {
                ram_bytes: daemon_memory_bytes(),
                ..Default::default()
            },
        };
        if times_ok && io_ok {
            *lock = Some(DaemonUsageSampler {
                sampled_at: now,
                cpu_100ns,
                io_read,
                io_write,
            });
        }
        usage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first sample only seeds the interval, so it reports the memory it can read and no CPU
    /// or disk rate; the next one has an interval and answers all three.
    #[test]
    fn test_daemon_usage_needs_an_interval_before_it_reports_cpu() {
        #[cfg(windows)]
        {
            *DAEMON_USAGE.lock().expect("the sampler lock is not poisoned") = None;
        }

        let first = sample_daemon_usage();
        assert!(first.ram_bytes > 0, "the private working set is always known");
        assert!(first.cpu_pct.is_none(), "no interval yet: no CPU percentage");
        assert_eq!(first.disk_bps(), 0, "no interval yet: no disk rate");

        std::thread::sleep(Duration::from_millis(30));
        let second = sample_daemon_usage();
        assert!(second.ram_bytes > 0);
        let cpu = second.cpu_pct.expect("a second sample has an interval");
        assert!((0.0..=100.0).contains(&cpu), "was {cpu}");
    }

    #[test]
    fn test_daemon_memory_reads_a_plausible_private_working_set() {
        let bytes = daemon_memory_bytes();
        assert!(bytes >= 1024 * 1024, "was {bytes}");
        assert!(bytes < 8 * 1024 * 1024 * 1024, "was {bytes}");
    }
}