use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::models::{TelemetryKind, TelemetryStyle};

#[cfg(windows)]
use super::audio::get_master_volume_state;
use super::engine::get_engine_telemetry;
#[cfg(windows)]
use super::process::get_target_process_metrics;

#[cfg(windows)]
use windows::Win32::Foundation::FILETIME;
#[cfg(windows)]
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX};
#[cfg(windows)]
use windows::Win32::System::Threading::GetSystemTimes;
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
pub(super) fn filetime_to_u64(ft: FILETIME) -> u64 {
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

pub(super) fn metrics_snapshot_is_fresh(sampled_at: Instant, target_matches: bool) -> bool {
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
