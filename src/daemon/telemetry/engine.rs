use std::sync::Mutex;

#[cfg(windows)]
use windows::Win32::System::SystemInformation::GetTickCount64;

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
