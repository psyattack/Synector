use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::models::{GraphMetric, TelemetryKind, TelemetryStyle};

#[cfg(windows)]
use windows::Win32::Foundation::FILETIME;
#[cfg(windows)]
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX};
#[cfg(windows)]
use windows::Win32::System::Threading::GetSystemTimes;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
#[cfg(windows)]
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};

#[cfg(windows)]
pub fn get_total_network_octets() -> Option<u64> {
    unsafe {
        let mut table_ptr: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table_ptr).is_ok() && !table_ptr.is_null() {
            let table = &*table_ptr;
            let num = table.NumEntries as usize;
            let rows_slice = std::slice::from_raw_parts(table.Table.as_ptr(), num);
            let mut total_octets: u64 = 0;
            for row in rows_slice {
                // Ignore loopback interfaces (IF_TYPE_SOFTWARE_LOOPBACK = 24)
                if row.Type != 24 {
                    total_octets = total_octets.saturating_add(row.InOctets).saturating_add(row.OutOctets);
                }
            }
            FreeMibTable(table_ptr as *const _);
            Some(total_octets)
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
pub fn get_total_network_octets() -> Option<u64> {
    None
}

struct NetworkSamplerState {
    last_sample: Instant,
    last_total_octets: u64,
    current_bytes_per_sec: f32,
}

static NETWORK_SAMPLER: Mutex<Option<NetworkSamplerState>> = Mutex::new(None);

pub fn get_network_speed_bytes_per_sec() -> f32 {
    let mut lock = match NETWORK_SAMPLER.lock() {
        Ok(g) => g,
        Err(_) => return 0.0,
    };

    let now = Instant::now();
    if let Some(ref mut st) = *lock {
        let elapsed = now.duration_since(st.last_sample);
        if elapsed.as_millis() >= 300 {
            if let Some(octets) = get_total_network_octets() {
                let diff = octets.saturating_sub(st.last_total_octets);
                let speed = diff as f32 / elapsed.as_secs_f32().max(0.001);
                // Exponential moving average for smooth display
                st.current_bytes_per_sec = st.current_bytes_per_sec * 0.25 + speed * 0.75;
                st.last_total_octets = octets;
                st.last_sample = now;
            }
        }
        st.current_bytes_per_sec
    } else {
        let octets = get_total_network_octets().unwrap_or(0);
        *lock = Some(NetworkSamplerState {
            last_sample: now,
            last_total_octets: octets,
            current_bytes_per_sec: 0.0,
        });
        0.0
    }
}

// ---------------- MEDIA PLAYBACK PROGRESS ----------------

// Track position and duration are only available through a media-session integration
// (GSMTC/SMTC), which this build does not consume. Nothing is synthesised from wall-clock time:
// media overlays show the current title/artist and transport controls only.

#[derive(Debug, Clone)]
pub struct SystemTelemetryData {
    pub cpu_usage_pct: f32,
    pub ram_usage_pct: f32,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub battery_pct: Option<u8>,
    pub is_ac_online: bool,
    pub uptime_secs: u64,
    pub master_volume: Option<u8>,
    pub is_muted: Option<bool>,
    pub target_process_ram_mb: Option<u64>,
    pub target_process_cpu_pct: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct EngineTelemetrySnapshot {
    pub is_daemon_alive: bool,
    pub heartbeat_pulse: bool,
    pub active_rules_count: usize,
    pub active_macros_count: usize,
    pub quota_status: String,
}

struct EngineStats {
    active_rules: usize,
    active_macros: usize,
    quota_rem: Option<u32>,
    quota_total: Option<u32>,
}

static ENGINE_STATS: Mutex<Option<EngineStats>> = Mutex::new(None);

pub fn update_engine_stats(active_rules: usize, active_macros: usize, quota_rem: Option<u32>, quota_total: Option<u32>) {
    if let Ok(mut lock) = ENGINE_STATS.lock() {
        *lock = Some(EngineStats {
            active_rules,
            active_macros,
            quota_rem,
            quota_total,
        });
    }
}

pub fn get_engine_telemetry() -> EngineTelemetrySnapshot {
    #[cfg(windows)]
    let uptime = unsafe { GetTickCount64() };
    #[cfg(not(windows))]
    let uptime = 0u64;

    let heartbeat_pulse = (uptime / 500) % 2 == 0;

    let lock = ENGINE_STATS.lock().ok();
    let stats = lock.as_ref().and_then(|g| g.as_ref());

    let (active_rules_count, active_macros_count, quota_status) = if let Some(st) = stats {
        let q = match (st.quota_rem, st.quota_total) {
            (Some(rem), Some(tot)) => format!("Quota: {}/{}", rem, tot),
            (Some(rem), None) => format!("Quota: {}", rem),
            _ => "Quota: Unlimited".to_string(),
        };
        (st.active_rules, st.active_macros, q)
    } else {
        (0, 0, "Quota: 100%".to_string())
    };

    EngineTelemetrySnapshot {
        is_daemon_alive: true,
        heartbeat_pulse,
        active_rules_count,
        active_macros_count,
        quota_status,
    }
}

// ---------------- SYSTEM METRICS SAMPLER ----------------

struct CpuSamplerState {
    last_sample: Instant,
    last_idle: u64,
    last_kernel: u64,
    last_user: u64,
    cached_cpu: f32,
}

static CPU_SAMPLER: Mutex<Option<CpuSamplerState>> = Mutex::new(None);

#[cfg(windows)]
fn filetime_to_u64(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64)
}

pub fn sample_system_cpu() -> f32 {
    #[cfg(windows)]
    {
        let mut sampler_lock = match CPU_SAMPLER.lock() {
            Ok(g) => g,
            Err(_) => return 15.0,
        };

        let now = Instant::now();
        if let Some(ref mut state) = *sampler_lock {
            if now.duration_since(state.last_sample).as_millis() < 300 {
                return state.cached_cpu;
            }

            let mut idle = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            unsafe {
                let _ = GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user));
            }

            let idle_u64 = filetime_to_u64(idle);
            let kernel_u64 = filetime_to_u64(kernel);
            let user_u64 = filetime_to_u64(user);

            let d_idle = idle_u64.saturating_sub(state.last_idle);
            let d_kernel = kernel_u64.saturating_sub(state.last_kernel);
            let d_user = user_u64.saturating_sub(state.last_user);
            let total = d_kernel + d_user;

            let cpu = if total > 0 {
                let busy = total.saturating_sub(d_idle);
                ((busy as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as f32
            } else {
                state.cached_cpu
            };

            state.last_sample = now;
            state.last_idle = idle_u64;
            state.last_kernel = kernel_u64;
            state.last_user = user_u64;
            state.cached_cpu = cpu;
            cpu
        } else {
            let mut idle = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            unsafe {
                let _ = GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user));
            }
            *sampler_lock = Some(CpuSamplerState {
                last_sample: now,
                last_idle: filetime_to_u64(idle),
                last_kernel: filetime_to_u64(kernel),
                last_user: filetime_to_u64(user),
                cached_cpu: 12.0,
            });
            12.0
        }
    }
    #[cfg(not(windows))]
    {
        20.0
    }
}

// ---------------- SYSTEM METRICS SNAPSHOT CACHE ----------------

struct MetricsSnapshot {
    sampled_at: Instant,
    target: Option<String>,
    data: SystemTelemetryData,
}

/// Overlay frame rate must not drive a Toolhelp snapshot + OpenProcess per element: graph samples,
/// the current label and formatted telemetry all read this one snapshot per target.
const METRICS_SNAPSHOT_TTL: Duration = Duration::from_millis(400);

static METRICS_SNAPSHOT: Mutex<Option<MetricsSnapshot>> = Mutex::new(None);

fn metrics_snapshot_is_fresh(sampled_at: Instant, target_matches: bool) -> bool {
    target_matches && sampled_at.elapsed() < METRICS_SNAPSHOT_TTL
}

pub fn get_system_metrics(target_process: Option<&str>) -> SystemTelemetryData {
    let target_key = target_process.map(|target| target.to_string());

    if let Ok(lock) = METRICS_SNAPSHOT.lock() {
        if let Some(cached) = lock.as_ref() {
            if metrics_snapshot_is_fresh(cached.sampled_at, cached.target == target_key) {
                return cached.data.clone();
            }
        }
    }

    let data = sample_system_metrics(target_process);
    if let Ok(mut lock) = METRICS_SNAPSHOT.lock() {
        *lock = Some(MetricsSnapshot {
            sampled_at: Instant::now(),
            target: target_key,
            data: data.clone(),
        });
    }
    data
}

#[cfg(windows)]
fn sample_system_metrics(target_process: Option<&str>) -> SystemTelemetryData {
    unsafe {
        let cpu_usage_pct = sample_system_cpu();

        let mut mem = MEMORYSTATUSEX::default();
        mem.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        let _ = GlobalMemoryStatusEx(&mut mem);
        let ram_usage_pct = mem.dwMemoryLoad as f32;
        let ram_total_mb = mem.ullTotalPhys / (1024 * 1024);
        let ram_used_mb = (mem.ullTotalPhys.saturating_sub(mem.ullAvailPhys)) / (1024 * 1024);

        let mut pwr = SYSTEM_POWER_STATUS::default();
        let _ = GetSystemPowerStatus(&mut pwr);
        let battery_pct = if pwr.BatteryLifePercent <= 100 {
            Some(pwr.BatteryLifePercent)
        } else {
            None
        };
        let is_ac_online = pwr.ACLineStatus == 1;

        let uptime_secs = GetTickCount64() / 1000;

        let (target_process_ram_mb, target_process_cpu_pct) = if let Some(target) = target_process {
            get_target_process_metrics(target)
        } else {
            (None, None)
        };

        let (master_volume, is_muted) = match get_master_volume_state() {
            Some((volume, muted)) => (Some(volume), Some(muted)),
            None => (None, None),
        };

        SystemTelemetryData {
            cpu_usage_pct,
            ram_usage_pct,
            ram_used_mb,
            ram_total_mb,
            battery_pct,
            is_ac_online,
            uptime_secs,
            master_volume,
            is_muted,
            target_process_ram_mb,
            target_process_cpu_pct,
        }
    }
}

#[cfg(not(windows))]
fn sample_system_metrics(_target_process: Option<&str>) -> SystemTelemetryData {
    SystemTelemetryData {
        cpu_usage_pct: 0.0,
        ram_usage_pct: 0.0,
        ram_used_mb: 0,
        ram_total_mb: 0,
        battery_pct: None,
        is_ac_online: false,
        uptime_secs: 0,
        master_volume: None,
        is_muted: None,
        target_process_ram_mb: None,
        target_process_cpu_pct: None,
    }
}

#[cfg(windows)]
fn get_target_process_metrics(target_name: &str) -> (Option<u64>, Option<f32>) {
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

fn process_cpu_percent(delta_100ns: u64, elapsed: Duration, cores: u32) -> Option<f32> {
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

/// Reads the default render endpoint's master volume and mute state. `None` means the endpoint
/// could not be queried (no audio device, COM unavailable), which overlays must display as n/a.
#[cfg(windows)]
fn get_master_volume_state() -> Option<(u8, bool)> {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

    with_com_apartment(|| {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()?;
        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) }.ok()?;
        let endpoint = unsafe { device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }.ok()?;
        let scalar = unsafe { endpoint.GetMasterVolumeLevelScalar() }.ok()?;
        let muted = unsafe { endpoint.GetMute() }.map(|flag| flag.as_bool()).unwrap_or(false);
        let volume = (scalar * 100.0).round().clamp(0.0, 100.0) as u8;
        Some((volume, muted))
    })
}

#[cfg(not(windows))]
fn get_master_volume_state() -> Option<(u8, bool)> {
    None
}

/// Runs `f` inside an MTA apartment. `CoUninitialize` is called only when this call actually
/// initialised COM (S_OK); S_FALSE and RPC_E_CHANGED_MODE mean another component owns the
/// apartment and must not be decremented here.
#[cfg(windows)]
fn with_com_apartment<T>(f: impl FnOnce() -> T) -> T {
    use windows::Win32::Foundation::S_OK;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

    unsafe {
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        let must_uninitialize = hr == S_OK;
        let result = f();
        if must_uninitialize {
            CoUninitialize();
        }
        result
    }
}

// ---------------- METRIC HISTORY RING BUFFERS ----------------

struct MetricBuffer {
    last_push: Instant,
    samples: VecDeque<f32>,
}

struct MetricHistoryTracker {
    buffers: HashMap<GraphMetric, MetricBuffer>,
}

const METRIC_PUSH_INTERVAL: Duration = Duration::from_millis(400);

static METRIC_HISTORY: Mutex<Option<MetricHistoryTracker>> = Mutex::new(None);

pub fn get_graph_samples(metric: GraphMetric, sample_count: usize, target_process: Option<&str>) -> Vec<f32> {
    let count = sample_count.clamp(8, 100);
    let metrics = get_system_metrics(target_process);

    // Metrics without a real source produce no history instead of a fabricated line.
    let current_val = match metric {
        GraphMetric::SystemCpu => Some(metrics.cpu_usage_pct / 100.0),
        GraphMetric::ProcessCpu => metrics.target_process_cpu_pct.map(|cpu| cpu / 100.0),
        GraphMetric::SystemRam => Some(metrics.ram_usage_pct / 100.0),
        GraphMetric::ProcessRam => metrics
            .target_process_ram_mb
            .map(|mb| (mb as f32 / metrics.ram_total_mb.max(1) as f32).clamp(0.0, 1.0)),
        // No frame-rate source is available in the daemon; the label reports n/a.
        GraphMetric::Fps => None,
        GraphMetric::Battery => metrics.battery_pct.map(|pct| pct as f32 / 100.0),
        GraphMetric::Network => {
            let speed = get_network_speed_bytes_per_sec();
            Some(if speed <= 10.0 {
                0.03
            } else {
                ((speed.log10() - 1.0) / 6.0).clamp(0.04, 0.96)
            })
        }
    };
    let Some(current_val) = current_val else {
        return Vec::new();
    };

    let mut lock = match METRIC_HISTORY.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };

    let now = Instant::now();
    let tracker = lock.get_or_insert_with(|| MetricHistoryTracker {
        buffers: HashMap::new(),
    });

    // Each metric has its own sample latch: two graphs in one frame must both advance.
    let buffer = tracker
        .buffers
        .entry(metric)
        .or_insert_with(|| MetricBuffer {
            last_push: now,
            samples: VecDeque::new(),
        });

    if buffer.samples.is_empty() || now.duration_since(buffer.last_push) >= METRIC_PUSH_INTERVAL {
        buffer.last_push = now;
        buffer.samples.push_back(current_val.clamp(0.02, 0.98));
        while buffer.samples.len() > 100 {
            buffer.samples.pop_front();
        }
    }

    // A short history is rendered short rather than padded with generated samples.
    buffer.samples.iter().rev().take(count).rev().copied().collect()
}

pub fn get_graph_current_label(metric: GraphMetric, target_process: Option<&str>) -> String {
    let metrics = get_system_metrics(target_process);
    match metric {
        GraphMetric::SystemCpu => format!("CPU: {:.0}%", metrics.cpu_usage_pct),
        GraphMetric::ProcessCpu => match metrics.target_process_cpu_pct {
            Some(cpu) => format!("P-CPU: {:.0}%", cpu),
            None => "P-CPU: n/a".to_string(),
        },
        GraphMetric::SystemRam => format!("RAM: {:.0}% ({:.1}/{:.1} GB)", metrics.ram_usage_pct, metrics.ram_used_mb as f32 / 1024.0, metrics.ram_total_mb as f32 / 1024.0),
        GraphMetric::ProcessRam => match metrics.target_process_ram_mb {
            Some(mb) => format!("P-RAM: {} MB", mb),
            None => "P-RAM: n/a".to_string(),
        },
        GraphMetric::Fps => "FPS: n/a".to_string(),
        GraphMetric::Battery => {
            if let Some(bat) = metrics.battery_pct {
                format!("BAT: {}%", bat)
            } else {
                "PWR: AC".to_string()
            }
        }
        GraphMetric::Network => {
            let speed = get_network_speed_bytes_per_sec();
            if speed >= 1024.0 * 1024.0 {
                format!("NET: {:.1} MB/s", speed / (1024.0 * 1024.0))
            } else if speed >= 1024.0 {
                format!("NET: {:.0} KB/s", speed / 1024.0)
            } else {
                format!("NET: {:.0} B/s", speed)
            }
        }
    }
}

// ---------------- SYSTEM TELEMETRY FORMATTER ----------------

pub fn format_telemetry_text(kind: TelemetryKind, style: TelemetryStyle, target_process: Option<&str>) -> String {
    if kind == TelemetryKind::EngineStatus {
        let engine = get_engine_telemetry();
        let pulse = if engine.heartbeat_pulse { "●" } else { "○" };
        return match style {
            TelemetryStyle::Compact => {
                format!("{} ONLINE | {} RUN", pulse, engine.active_rules_count)
            }
            TelemetryStyle::Badge => {
                format!("{} SYNECTOR: ONLINE ({} active) | {}", pulse, engine.active_rules_count, engine.quota_status)
            }
            TelemetryStyle::Detailed => {
                let status = if engine.is_daemon_alive { "Online" } else { "Offline" };
                format!("Synector {}: Pulse 1s | Rules: {} | Macros: {} | {}", status, engine.active_rules_count, engine.active_macros_count, engine.quota_status)
            }
        };
    }

    let metrics = get_system_metrics(target_process);

    match kind {
        TelemetryKind::Battery => {
            let pwr_icon = if metrics.is_ac_online { "⚡" } else { "🔋" };
            let pct_str = metrics
                .battery_pct
                .map(|p| format!("{}%", p))
                .unwrap_or_else(|| "n/a".to_string());
            match style {
                TelemetryStyle::Compact => format!("{} {}", pwr_icon, pct_str),
                TelemetryStyle::Badge => {
                    if metrics.is_ac_online {
                        format!("⚡ {} [AC ONLINE]", pct_str)
                    } else {
                        format!("🔋 {} [BATTERY]", pct_str)
                    }
                }
                TelemetryStyle::Detailed => {
                    if metrics.is_ac_online {
                        format!("Power: {} (AC Connected, Uptime: {}h)", pct_str, metrics.uptime_secs / 3600)
                    } else {
                        format!("Battery: {} (Discharging, Uptime: {}h)", pct_str, metrics.uptime_secs / 3600)
                    }
                }
            }
        }
        TelemetryKind::AudioMicStatus => {
            let is_muted = metrics.is_muted == Some(true);
            let vol_icon = if is_muted { "🔇" } else { "🔊" };
            let volume = metrics
                .master_volume
                .map(|volume| format!("{}%", volume))
                .unwrap_or_else(|| "n/a".to_string());
            match style {
                TelemetryStyle::Compact => {
                    if is_muted {
                        "MUTE".to_string()
                    } else {
                        format!("VOL: {}", volume)
                    }
                }
                TelemetryStyle::Badge => {
                    if is_muted {
                        "🔇 MUTE".to_string()
                    } else {
                        format!("{} VOL: {}", vol_icon, volume)
                    }
                }
                TelemetryStyle::Detailed => {
                    if is_muted {
                        "Audio: Muted".to_string()
                    } else {
                        format!("Master Volume: {}", volume)
                    }
                }
            }
        }
        TelemetryKind::EngineStatus => unreachable!("engine status is formatted above"),
    }
}

// ---------------- REAL KEY VISUALIZER ----------------

static LAST_KEYSTROKE: Mutex<Option<(Instant, String)>> = Mutex::new(None);

#[cfg(windows)]
#[inline]
pub fn is_vk_pressed(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
}

#[cfg(not(windows))]
#[inline]
pub fn is_vk_pressed(_vk: i32) -> bool {
    false
}

pub fn get_active_or_recent_keys(max_keys: usize, decay_sec: f32) -> String {
    #[cfg(windows)]
    {
        let mut pressed: Vec<String> = Vec::new();
        unsafe {
            if GetAsyncKeyState(0x11) as u16 & 0x8000 != 0 { pressed.push("CTRL".to_string()); }
            if GetAsyncKeyState(0x10) as u16 & 0x8000 != 0 { pressed.push("SHIFT".to_string()); }
            if GetAsyncKeyState(0x12) as u16 & 0x8000 != 0 { pressed.push("ALT".to_string()); }
            if GetAsyncKeyState(0x5B) as u16 & 0x8000 != 0 { pressed.push("WIN".to_string()); }
            if GetAsyncKeyState(0x20) as u16 & 0x8000 != 0 { pressed.push("SPACE".to_string()); }
            if GetAsyncKeyState(0x0D) as u16 & 0x8000 != 0 { pressed.push("ENTER".to_string()); }
            if GetAsyncKeyState(0x1B) as u16 & 0x8000 != 0 { pressed.push("ESC".to_string()); }
            if GetAsyncKeyState(0x09) as u16 & 0x8000 != 0 { pressed.push("TAB".to_string()); }

            // Letter keys A-Z (0x41 ..= 0x5A)
            for vk in 0x41u8..=0x5Au8 {
                if GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 {
                    pressed.push(format!("{}", vk as char));
                }
            }

            // Function keys F1-F12 (0x70 ..= 0x7B)
            for vk in 0x70u8..=0x7Bu8 {
                if GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 {
                    let num = vk - 0x70 + 1;
                    pressed.push(format!("F{}", num));
                }
            }
        }

        let now = Instant::now();
        if !pressed.is_empty() {
            let key_str = pressed.iter().take(max_keys).map(|k| format!("[{}]", k)).collect::<Vec<_>>().join(" + ");
            if let Ok(mut lock) = LAST_KEYSTROKE.lock() {
                *lock = Some((now, key_str.clone()));
            }
            return key_str;
        }

        if let Ok(lock) = LAST_KEYSTROKE.lock() {
            if let Some((time, ref key_str)) = *lock {
                if now.duration_since(time).as_secs_f32() <= decay_sec {
                    return key_str.clone();
                }
            }
        }

        "[KEYS IDLE]".to_string()
    }
    #[cfg(not(windows))]
    {
        let _ = (max_keys, decay_sec);
        "[KEYS IDLE]".to_string()
    }
}

// ---------------- MEDIA HELPER ----------------

/// Volume and media keys are E0-prefixed; they need the extended flag in addition to the scan
/// code so players receive a real WM_APPCOMMAND.
fn media_key_is_extended(vk: u8) -> bool {
    matches!(vk, 0xA6..=0xB7)
}

pub fn simulate_media_key(vk: u8) {
    if !crate::daemon::actions::INPUT_INJECTION_ENABLED {
        return;
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
            KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
        };

        let scan = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16;
        let mut flags = KEYBD_EVENT_FLAGS(0);
        if scan != 0 {
            flags |= KEYEVENTF_SCANCODE;
        }
        if media_key_is_extended(vk) {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }

        let down = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk as u16),
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let mut up = down;
        up.Anonymous.ki.dwFlags |= KEYEVENTF_KEYUP;

        let _ = SendInput(&[down], std::mem::size_of::<INPUT>() as i32);
        // Hold long enough for DirectInput-style players to register the press.
        std::thread::sleep(Duration::from_millis(35));
        let _ = SendInput(&[up], std::mem::size_of::<INPUT>() as i32);
    }
    #[cfg(not(windows))]
    {
        let _ = vk;
    }
}


// ---------------- ADVANCED MEDIA STATUS DETECTION ----------------

pub fn is_system_audio_playing() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
        use windows::Win32::Media::Audio::{eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

        with_com_apartment(|| {
            let mut peak = 0.0f32;
            if let Ok(enumerator) = unsafe {
                CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
            } {
                if let Ok(device) = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) } {
                    if let Ok(meter) = unsafe { device.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None) } {
                        if let Ok(value) = unsafe { meter.GetPeakValue() } {
                            peak = value;
                        }
                    }
                }
            }
            peak > 0.0005
        })
    }
    #[cfg(not(windows))]
    false
}

#[derive(Debug, Clone)]
pub struct SystemMediaStatus {
    pub is_playing: bool,
    pub title: String,
    pub artist: String,
    pub player_name: String,
}

static CACHED_MEDIA: Mutex<Option<(Instant, SystemMediaStatus)>> = Mutex::new(None);

pub fn get_system_media_status() -> SystemMediaStatus {
    if let Ok(lock) = CACHED_MEDIA.lock() {
        if let Some((time, ref status)) = *lock {
            if time.elapsed() < std::time::Duration::from_millis(200) {
                return status.clone();
            }
        }
    }

    #[cfg(windows)]
    {
        use windows::core::BOOL;
        use windows::Win32::Foundation::{HWND, LPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible, GetWindowThreadProcessId,
        };

        struct EnumContext {
            found: Option<SystemMediaStatus>,
        }

        unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = &mut *(lparam.0 as *mut EnumContext);
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            let len = GetWindowTextLengthW(hwnd);
            if !(3..=512).contains(&len) {
                return BOOL(1);
            }
            let mut buf = vec![0u16; (len + 1) as usize];
            let read_len = GetWindowTextW(hwnd, &mut buf);
            if read_len <= 0 {
                return BOOL(1);
            }
            let full_title = String::from_utf16_lossy(&buf[..read_len as usize]).trim().to_string();
            if full_title.is_empty() {
                return BOOL(1);
            }

            // Check YouTube
            if let Some(pos) = full_title.find(" - YouTube") {
                let track_part = full_title[..pos].trim();
                let is_nav = track_part.eq_ignore_ascii_case("youtube")
                    || track_part.eq_ignore_ascii_case("home")
                    || track_part.eq_ignore_ascii_case("subscriptions")
                    || track_part.eq_ignore_ascii_case("feed")
                    || track_part.is_empty();
                if !is_nav {
                    let (artist, title) = if let Some((a, t)) = track_part.split_once(" - ") {
                        (a.trim().to_string(), t.trim().to_string())
                    } else {
                        ("YouTube".to_string(), track_part.to_string())
                    };
                    let is_active = is_system_audio_playing();
                    ctx.found = Some(SystemMediaStatus {
                        is_playing: is_active,
                        title,
                        artist,
                        player_name: "YouTube".to_string(),
                    });
                    return BOOL(0);
                }
            }

            // Check SoundCloud
            if let Some(pos) = full_title.find(" | SoundCloud") {
                let track_part = full_title[..pos].trim();
                let (artist, title) = if let Some((a, t)) = track_part.split_once(" - ") {
                    (a.trim().to_string(), t.trim().to_string())
                } else {
                    ("SoundCloud".to_string(), track_part.to_string())
                };
                let is_active = is_system_audio_playing();
                ctx.found = Some(SystemMediaStatus {
                    is_playing: is_active,
                    title,
                    artist,
                    player_name: "SoundCloud".to_string(),
                });
                return BOOL(0);
            }

            // Check Yandex Music
            if full_title.contains("Яндекс Музыка") || full_title.contains("Яндекс.Музыка") {
                let clean = full_title
                    .replace("— Яндекс Музыка", "")
                    .replace("- Яндекс Музыка", "")
                    .replace("— Яндекс.Музыка", "")
                    .replace("- Яндекс.Музыка", "")
                    .trim()
                    .to_string();
                let (artist, title) = if let Some((a, t)) = clean.split_once(" — ") {
                    (a.trim().to_string(), t.trim().to_string())
                } else if let Some((a, t)) = clean.split_once(" - ") {
                    (a.trim().to_string(), t.trim().to_string())
                } else {
                    ("Yandex Music".to_string(), clean)
                };
                let is_active = is_system_audio_playing();
                ctx.found = Some(SystemMediaStatus {
                    is_playing: is_active,
                    title,
                    artist,
                    player_name: "Yandex Music".to_string(),
                });
                return BOOL(0);
            }

            // Check VLC
            if let Some(pos) = full_title.find(" - VLC media player") {
                let track_part = full_title[..pos].trim();
                let (artist, title) = if let Some((a, t)) = track_part.split_once(" - ") {
                    (a.trim().to_string(), t.trim().to_string())
                } else {
                    ("VLC Media Player".to_string(), track_part.to_string())
                };
                let is_active = is_system_audio_playing();
                ctx.found = Some(SystemMediaStatus {
                    is_playing: is_active,
                    title,
                    artist,
                    player_name: "VLC".to_string(),
                });
                return BOOL(0);
            }

            // Check AIMP
            if let Some(pos) = full_title.find(" - AIMP") {
                let track_part = full_title[..pos].trim();
                let (artist, title) = if let Some((a, t)) = track_part.split_once(" - ") {
                    (a.trim().to_string(), t.trim().to_string())
                } else {
                    ("AIMP Player".to_string(), track_part.to_string())
                };
                let is_active = is_system_audio_playing();
                ctx.found = Some(SystemMediaStatus {
                    is_playing: is_active,
                    title,
                    artist,
                    player_name: "AIMP".to_string(),
                });
                return BOOL(0);
            }

            // Check Spotify: the window title alone is ambiguous, so confirm the owner process
            // with one image-path query instead of enumerating all processes per window.
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid != 0 {
                let is_spotify_idle = full_title.eq_ignore_ascii_case("spotify") 
                    || full_title.eq_ignore_ascii_case("spotify free") 
                    || full_title.eq_ignore_ascii_case("spotify premium");
                
                if !is_spotify_idle && full_title.contains(" - ") {
                    let is_spotify = crate::daemon::watcher::ProcessWatcher::get_process_image_path(pid)
                        .as_deref()
                        .and_then(|path| std::path::Path::new(path).file_name().and_then(|name| name.to_str()))
                        .is_some_and(|name| name.eq_ignore_ascii_case("spotify.exe"));

                    if is_spotify {
                        let (artist, title) = if let Some((a, t)) = full_title.split_once(" - ") {
                            (a.trim().to_string(), t.trim().to_string())
                        } else {
                            ("Spotify".to_string(), full_title)
                        };
                        let is_active = is_system_audio_playing();
                        ctx.found = Some(SystemMediaStatus {
                            is_playing: is_active,
                            title,
                            artist,
                            player_name: "Spotify".to_string(),
                        });
                        return BOOL(0);
                    }
                }
            }

            BOOL(1)
        }

        let mut context = EnumContext { found: None };
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut context as *mut _ as isize));
        }

        let status = context.found.unwrap_or_else(|| SystemMediaStatus {
            is_playing: false,
            title: "No Track Playing".to_string(),
            artist: "Synector Media".to_string(),
            player_name: "Media".to_string(),
        });

        if let Ok(mut lock) = CACHED_MEDIA.lock() {
            *lock = Some((Instant::now(), status.clone()));
        }
        status
    }
    #[cfg(not(windows))]
    {
        SystemMediaStatus {
            is_playing: false,
            title: "No Track Playing".to_string(),
            artist: "Synector Media".to_string(),
            player_name: "Media".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_cpu_percent_math() {
        // One fully busy core for one second is 100 % of a single-core machine.
        let one_core = process_cpu_percent(10_000_000, Duration::from_secs(1), 1).unwrap();
        assert!((one_core - 100.0).abs() < 0.01);

        // The same CPU time spread over four cores is 25 % of total capacity.
        let four_cores = process_cpu_percent(10_000_000, Duration::from_secs(1), 4).unwrap();
        assert!((four_cores - 25.0).abs() < 0.01);

        assert_eq!(process_cpu_percent(10, Duration::ZERO, 4), None);
        assert_eq!(process_cpu_percent(10, Duration::from_secs(1), 0), None);
    }

    #[test]
    fn test_metric_history_advances_per_metric() {
        if let Ok(mut lock) = METRIC_HISTORY.lock() {
            *lock = None;
        }

        let cpu_first = get_graph_samples(GraphMetric::SystemCpu, 8, None);
        let ram_first = get_graph_samples(GraphMetric::SystemRam, 8, None);
        assert_eq!(cpu_first.len(), 1);
        assert_eq!(ram_first.len(), 1);

        std::thread::sleep(Duration::from_millis(410));

        let cpu_second = get_graph_samples(GraphMetric::SystemCpu, 8, None);
        let ram_second = get_graph_samples(GraphMetric::SystemRam, 8, None);
        assert_eq!(cpu_second.len(), 2, "the CPU buffer must advance");
        assert_eq!(
            ram_second.len(),
            2,
            "a second graph must advance even when another metric sampled in the same frame"
        );
        assert!(cpu_second.iter().all(|value| value.is_finite()));
        assert!(ram_second.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn test_unsupported_graph_metrics_return_no_samples() {
        assert!(get_graph_samples(GraphMetric::Fps, 16, None).is_empty());
        assert!(get_graph_samples(GraphMetric::ProcessCpu, 16, None).is_empty());
        assert!(get_graph_samples(GraphMetric::ProcessRam, 16, None).is_empty());

        assert_eq!(get_graph_current_label(GraphMetric::Fps, None), "FPS: n/a");
        assert_eq!(
            get_graph_current_label(GraphMetric::ProcessCpu, None),
            "P-CPU: n/a"
        );
        assert_eq!(
            get_graph_current_label(GraphMetric::ProcessRam, None),
            "P-RAM: n/a"
        );
    }

    #[test]
    fn test_media_key_extended_flags() {
        assert!(media_key_is_extended(0xAD)); // VK_VOLUME_MUTE
        assert!(media_key_is_extended(0xAF)); // VK_VOLUME_UP
        assert!(media_key_is_extended(0xB3)); // VK_MEDIA_PLAY_PAUSE
        assert!(!media_key_is_extended(b'A'));
    }

    #[test]
    fn test_metrics_snapshot_cache_freshness() {
        assert!(metrics_snapshot_is_fresh(Instant::now(), true));
        assert!(!metrics_snapshot_is_fresh(Instant::now(), false));
        assert!(!metrics_snapshot_is_fresh(
            Instant::now() - Duration::from_millis(800),
            true
        ));
    }
}
