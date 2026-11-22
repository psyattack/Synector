//! Rule condition evaluation and process matching.
//!
//! Actions evaluate rule conditions and process names inside spawned pipelines, where the
//! daemon's `Arc<Mutex<DaemonEngine>>` is not available. The OS probes therefore live here as
//! free functions: action handlers call them directly, and the engine's trigger and pipeline
//! paths reach the same code through the `DaemonEngine` facade.

use crate::daemon::watcher::ProcessWatcher;
use crate::models::{ConditionOperator, ProcessMatchMode, RuleCondition, RuleConditionItem};

#[cfg(windows)]
pub(crate) fn get_current_foreground_process() -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let hwnd = unsafe { GetForegroundWindow() };
    if !hwnd.0.is_null() {
        let mut fg_pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut fg_pid)) };
        if fg_pid != 0 {
            let snap = ProcessWatcher::snapshot_process_map();
            return snap.get(&fg_pid).cloned();
        }
    }
    None
}
#[cfg(not(windows))]
pub(crate) fn get_current_foreground_process() -> Option<String> {
    None
}

static REGEX_CACHE: std::sync::LazyLock<std::sync::RwLock<std::collections::HashMap<String, Option<regex::Regex>>>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(std::collections::HashMap::new()));

fn match_cached_regex(pattern: &str, actual: &str) -> bool {
    if let Ok(cache) = REGEX_CACHE.read() {
        if let Some(opt_re) = cache.get(pattern) {
            return match opt_re {
                Some(re) => re.is_match(actual),
                None => actual.to_lowercase().contains(&pattern.to_lowercase()),
            };
        }
    }

    let compiled = regex::RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .ok();

    let matched = match &compiled {
        Some(re) => re.is_match(actual),
        None => actual.to_lowercase().contains(&pattern.to_lowercase()),
    };

    if let Ok(mut cache) = REGEX_CACHE.write() {
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(pattern.to_string(), compiled);
    }

    matched
}

    #[inline]
    pub(crate) fn strip_exe_suffix(name: &str) -> &str {
        if name.len() >= 4 && name[name.len() - 4..].eq_ignore_ascii_case(".exe") {
            &name[..name.len() - 4]
        } else {
            name
        }
    }

    /// Matches an optional device filter against the interface path reported by Windows.
    /// An empty filter accepts any device; otherwise the filter is a case-insensitive
    /// substring, which works for IDs (`VID_1234`), interface paths and drive letters.
    pub(crate) fn device_filter_matches(filter: Option<&str>, device_id: Option<&str>) -> bool {
        let Some(filter) = filter.map(str::trim).filter(|value| !value.is_empty()) else {
            return true;
        };
        if filter == "*" {
            return true;
        }
        let Some(device_id) = device_id else {
            return false;
        };
        device_id
            .to_lowercase()
            .contains(&filter.to_lowercase())
    }

    pub(crate) fn matches_process(pattern: &str, actual: &str, mode: ProcessMatchMode) -> bool {
        if pattern == "*" {
            return true;
        }
        match mode {
            ProcessMatchMode::Exact => {
                pattern.eq_ignore_ascii_case(actual)
                    || strip_exe_suffix(pattern).eq_ignore_ascii_case(strip_exe_suffix(actual))
            }
            ProcessMatchMode::Wildcard => {
                let match_wildcard = |p: &str, a: &str| -> bool {
                    let pat = p.to_lowercase();
                    let act = a.to_lowercase();
                    if pat.starts_with('*') && pat.ends_with('*') && pat.len() >= 2 {
                        act.contains(&pat[1..pat.len() - 1])
                    } else if let Some(stripped) = pat.strip_prefix('*') {
                        act.ends_with(stripped)
                    } else if let Some(stripped) = pat.strip_suffix('*') {
                        act.starts_with(stripped)
                    } else {
                        act == pat || strip_exe_suffix(&act) == strip_exe_suffix(&pat)
                    }
                };
                match_wildcard(pattern, actual) || match_wildcard(pattern, strip_exe_suffix(actual))
            }
            ProcessMatchMode::Regex => {
                match_cached_regex(pattern, actual) || match_cached_regex(pattern, strip_exe_suffix(actual))
            }
        }
    }

    pub(crate) fn evaluate_condition(condition: &RuleCondition) -> bool {
        match condition {
            RuleCondition::ProcessRunning { process_name, match_mode, must_be_running } => {
                let snap = ProcessWatcher::snapshot_process_map();
                let is_running = snap.values().any(|name| matches_process(process_name, name, *match_mode));
                is_running == *must_be_running
            }
            RuleCondition::WindowFocused { process_name, match_mode, must_be_focused } => {
                let fg_name = get_current_foreground_process();
                let is_focused = fg_name.as_deref().is_some_and(|name| matches_process(process_name, name, *match_mode));
                is_focused == *must_be_focused
            }
            RuleCondition::TimeRange { start_time, end_time, days_of_week } => {
                let now = chrono::Local::now();
                if !days_of_week.is_empty() {
                    use chrono::Datelike;
                    let dow = now.weekday().number_from_monday() as u8;
                    if !days_of_week.contains(&dow) {
                        return false;
                    }
                }
                if start_time.is_empty() && end_time.is_empty() {
                    return true;
                }
                let cur_hm = now.format("%H:%M").to_string();
                let st = if start_time.is_empty() { "00:00" } else { start_time.as_str() };
                let et = if end_time.is_empty() { "23:59" } else { end_time.as_str() };
                if st <= et {
                    cur_hm.as_str() >= st && cur_hm.as_str() <= et
                } else {
                    cur_hm.as_str() >= st || cur_hm.as_str() <= et
                }
            }
            RuleCondition::Event { event_type, idle_duration_sec } => {
                match event_type {
                    crate::models::SystemEventType::PowerSourceAc => {
                        crate::daemon::system_events::is_ac_power().unwrap_or(true)
                    }
                    crate::models::SystemEventType::PowerSourceBattery => {
                        !crate::daemon::system_events::is_ac_power().unwrap_or(true)
                    }
                    crate::models::SystemEventType::UserIdle => {
                        let idle_s = crate::daemon::system_events::get_user_idle_seconds().unwrap_or(0);
                        idle_s >= (*idle_duration_sec as u64)
                    }
                    crate::models::SystemEventType::UserActive => {
                        let idle_s = crate::daemon::system_events::get_user_idle_seconds().unwrap_or(0);
                        idle_s < (*idle_duration_sec as u64)
                    }
                    crate::models::SystemEventType::SessionLock => {
                        crate::daemon::system_events::is_session_locked()
                    }
                    crate::models::SystemEventType::SessionUnlock => {
                        !crate::daemon::system_events::is_session_locked()
                    }
                    crate::models::SystemEventType::NetworkConnected => {
                        crate::daemon::system_events::is_network_connected()
                    }
                    crate::models::SystemEventType::NetworkDisconnected => {
                        !crate::daemon::system_events::is_network_connected()
                    }
                    crate::models::SystemEventType::SystemSleep => {
                        crate::daemon::system_events::is_system_sleeping()
                    }
                    crate::models::SystemEventType::SystemResume => {
                        !crate::daemon::system_events::is_system_sleeping()
                    }
                    crate::models::SystemEventType::DeviceConnected => {
                        crate::daemon::system_events::device_state_known()
                            && crate::daemon::system_events::has_connected_device()
                    }
                    crate::models::SystemEventType::DeviceDisconnected => {
                        crate::daemon::system_events::device_state_known()
                            && !crate::daemon::system_events::has_connected_device()
                    }
                    crate::models::SystemEventType::MonitorOn => {
                        crate::daemon::system_events::is_display_on()
                    }
                    crate::models::SystemEventType::MonitorOff => {
                        !crate::daemon::system_events::is_display_on()
                    }
                }
            }
            RuleCondition::RuleRunning { rule_id, must_be_running } => {
                let target = rule_id.trim();
                !target.is_empty()
                    && crate::daemon::rule_activity::is_running(target) == *must_be_running
            }
            RuleCondition::FileWatcher { path, filter_extension, must_exist } => {
                let p = std::path::Path::new(path);
                let exists = if p.is_dir() {
                    if let Some(ext) = filter_extension {
                        let ext_clean = ext.trim().trim_start_matches('*').trim_start_matches('.').to_lowercase();
                        if ext_clean.is_empty() {
                            p.exists()
                        } else if let Ok(entries) = std::fs::read_dir(p) {
                            entries.filter_map(|e| e.ok()).any(|e| {
                                let ep = e.path();
                                if let Some(file_ext) = ep.extension().and_then(|s| s.to_str()) {
                                    file_ext.eq_ignore_ascii_case(&ext_clean)
                                } else if let Some(file_name) = ep.file_name().and_then(|s| s.to_str()) {
                                    file_name.eq_ignore_ascii_case(&ext_clean)
                                } else {
                                    false
                                }
                            })
                        } else {
                            false
                        }
                    } else {
                        p.exists()
                    }
                } else if let Some(ext) = filter_extension {
                    let ext_clean = ext.trim().trim_start_matches('*').trim_start_matches('.').to_lowercase();
                    if ext_clean.is_empty() {
                        p.exists()
                    } else {
                        let matches_ext = p.extension()
                            .and_then(|s| s.to_str())
                            .map(|e| e.eq_ignore_ascii_case(&ext_clean))
                            .unwrap_or(false);
                        p.exists() && matches_ext
                    }
                } else {
                    p.exists()
                };
                exists == *must_exist
            }
            RuleCondition::ResourceThreshold { process_name, cpu_percent, ram_mb } => {
                let snap = ProcessWatcher::get_detailed_snapshot();
                for proc in &snap {
                    if matches_process(process_name, &proc.name, ProcessMatchMode::Exact) {
                        let mem_mb = proc.memory_bytes / (1024 * 1024);
                        let over_mem = ram_mb.is_none_or(|limit| mem_mb >= limit);
                        let over_cpu = cpu_percent.is_none_or(|limit| proc.cpu_percent >= limit);
                        if over_mem && over_cpu && (ram_mb.is_some() || cpu_percent.is_some()) {
                            return true;
                        }
                    }
                }
                false
            }
        }
    }

    pub(crate) fn evaluate_rule_conditions(conditions: &[RuleConditionItem]) -> bool {
        if conditions.is_empty() {
            return true;
        }

        let mut current_group_satisfied = true;
        let mut any_group_satisfied = false;

        for (i, item) in conditions.iter().enumerate() {
            let cond_val = evaluate_condition(&item.condition);
            if i == 0 {
                current_group_satisfied = cond_val;
            } else {
                match item.operator {
                    ConditionOperator::And => {
                        current_group_satisfied = current_group_satisfied && cond_val;
                    }
                    ConditionOperator::Or => {
                        if current_group_satisfied {
                            return true;
                        }
                        any_group_satisfied = any_group_satisfied || current_group_satisfied;
                        current_group_satisfied = cond_val;
                    }
                }
            }
        }

        any_group_satisfied || current_group_satisfied
    }
