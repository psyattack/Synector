use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::models::Rule;

/// Scheduler admission clocks. `rule_last_triggered` stays on [`DaemonEngine`] because the
/// daemon's hotkey-toggle path stamps it; the tick-level retry throttle and the
/// resource-threshold dwell tracker live here.
#[derive(Default)]
pub(super) struct Scheduler {
    /// Instant of the last scheduler *attempt* (accepted or rejected). It only throttles
    /// tick-level retries so a rejected run is not retried every 250 ms; the cooldown gate
    /// reads `rule_last_triggered`, which is stamped when a run is accepted.
    pub(super) last_attempt: HashMap<String, Instant>,
    /// Since when each `(rule, pid)` pair has been above its resource thresholds.
    resource_since: HashMap<(String, u32), Instant>,
}

impl Scheduler {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Drops every admitted-run clock of a rule that no longer exists.
    pub(super) fn forget_rule(&mut self, rule_id: &str) {
        self.last_attempt.remove(rule_id);
        self.resource_since
            .retain(|(id, _), _| id.as_str() != rule_id);
    }

    /// Interval schedules. A persisted `last_executed_at` seeds the accepted-run clock so the
    /// next run happens one interval later, and a rejected attempt is retried once per
    /// interval instead of every tick. Returns whether a run is due now.
    pub(super) fn interval_due(
        &mut self,
        rule: &Rule,
        last_triggered: &mut HashMap<String, Instant>,
        now_instant: Instant,
        now_local: chrono::DateTime<chrono::Local>,
        interval: u64,
        target_datetime: Option<&str>,
    ) -> bool {
        if interval == 0 {
            return false;
        }

        // If a specific first execution date & time is set, ensure current time >= target.
        if let Some(target) = target_datetime {
            let cur_datetime = now_local.format("%Y-%m-%d %H:%M:%S").to_string();
            let target_cmp = if target.len() == 16 {
                format!("{target}:00")
            } else {
                target.to_string()
            };
            if cur_datetime < target_cmp && !last_triggered.contains_key(&rule.id) {
                return false;
            }
        }

        let interval_due = match last_triggered.get(&rule.id) {
            Some(last) => last.elapsed().as_secs() >= interval,
            None => {
                if let Some(last_str) = &rule.last_executed_at {
                    match chrono::NaiveDateTime::parse_from_str(last_str, "%Y-%m-%d %H:%M:%S") {
                        Ok(last_dt) => {
                            let elapsed_secs = now_local
                                .naive_local()
                                .signed_duration_since(last_dt)
                                .num_seconds();
                            if elapsed_secs >= 0 && (elapsed_secs as u64) < interval {
                                // Seed the in-memory clock from the persisted run so the next
                                // one happens one interval later. This is not a trigger: the
                                // cooldown keeps measuring the last accepted run.
                                let virtual_start = now_instant
                                    - std::time::Duration::from_secs(elapsed_secs as u64);
                                last_triggered.insert(rule.id.clone(), virtual_start);
                                false
                            } else {
                                true
                            }
                        }
                        Err(_) => target_datetime.is_some(),
                    }
                } else if target_datetime.is_some() {
                    true
                } else {
                    // No run yet: the first tick starts the interval clock and later ticks
                    // measure the interval from that attempt.
                    match self.last_attempt.get(&rule.id).copied() {
                        Some(attempt) => attempt.elapsed().as_secs() >= interval,
                        None => {
                            self.last_attempt.insert(rule.id.clone(), now_instant);
                            false
                        }
                    }
                }
            }
        };

        // A rejected attempt is retried once per interval, not every 250 ms tick.
        let retry_throttled = last_triggered.contains_key(&rule.id)
            && self
                .last_attempt
                .get(&rule.id)
                .is_some_and(|attempt| attempt.elapsed().as_secs() < interval);

        if interval_due && !retry_throttled {
            self.last_attempt.insert(rule.id.clone(), now_instant);
            true
        } else {
            false
        }
    }

    /// Daily schedules: due after `daily_time` on a configured weekday, once per day, with
    /// rejected attempts retried after a short suppression window.
    pub(super) fn daily_due(
        &mut self,
        rule: &Rule,
        now_instant: Instant,
        now_local: chrono::DateTime<chrono::Local>,
        daily_time: &str,
        days_of_week: &[u8],
    ) -> bool {
        use chrono::Datelike;
        let current_dow = now_local.weekday().number_from_monday() as u8;
        if !days_of_week.is_empty() && !days_of_week.contains(&current_dow) {
            return false;
        }

        let today = now_local.format("%Y-%m-%d").to_string();
        let already_executed_today = rule
            .last_executed_at
            .as_deref()
            .is_some_and(|ts| ts.starts_with(&today));
        let retry_suppressed = self
            .last_attempt
            .get(&rule.id)
            .is_some_and(|attempt| attempt.elapsed().as_secs() < 3);
        if already_executed_today || retry_suppressed {
            return false;
        }

        let cur_time_hms = now_local.format("%H:%M:%S").to_string();
        let cur_time_hm = now_local.format("%H:%M").to_string();
        let target_time_str = if daily_time.len() == 5 {
            format!("{daily_time}:00")
        } else {
            daily_time.to_string()
        };
        let target_passed =
            cur_time_hms >= target_time_str || cur_time_hm.as_str() >= daily_time;

        if target_passed {
            // The accepted run stamps `rule_last_triggered`; stamping it here would make the
            // cooldown reject that very run.
            self.last_attempt.insert(rule.id.clone(), now_instant);
            true
        } else {
            false
        }
    }

    /// Monthly schedules: due from `day_of_month` at `daily_time` onward, once per month.
    pub(super) fn monthly_due(
        rule: &Rule,
        now_local: chrono::DateTime<chrono::Local>,
        day_of_month: Option<u8>,
        daily_time: Option<&str>,
    ) -> bool {
        use chrono::Datelike;
        let target_day = day_of_month.unwrap_or(1).clamp(1, 31);
        let target_time_str = daily_time.unwrap_or("12:00:00");
        let cur_year_month = now_local.format("%Y-%m").to_string();
        let cur_day = now_local.day() as u8;
        let cur_time_hms = now_local.format("%H:%M:%S").to_string();
        let cur_time_hm = now_local.format("%H:%M").to_string();

        let already_executed_this_month = rule
            .last_executed_at
            .as_deref()
            .is_some_and(|ts| ts.starts_with(&cur_year_month));
        if already_executed_this_month {
            return false;
        }

        if cur_day > target_day {
            true
        } else if cur_day == target_day {
            cur_time_hms.as_str() >= target_time_str
                || cur_time_hm.as_str() >= target_time_str
                || format!("{cur_time_hm}:00").as_str() >= target_time_str
        } else {
            false
        }
    }

    /// One-shot schedules: due from `target` onward until the run is accepted. A rejected run
    /// keeps the one-shot armed.
    pub(super) fn oneshot_due(
        rule: &Rule,
        last_triggered: &HashMap<String, Instant>,
        now_local: chrono::DateTime<chrono::Local>,
        target: &str,
    ) -> bool {
        let cur_datetime = now_local.format("%Y-%m-%d %H:%M:%S").to_string();
        let target_cmp = if target.len() == 16 {
            format!("{target}:00")
        } else {
            target.to_string()
        };
        cur_datetime >= target_cmp && !last_triggered.contains_key(&rule.id)
    }

    /// Drops resource-threshold dwell entries for rules that were disabled or processes that
    /// disappeared, so the trigger restarts its dwell from a fresh sighting.
    pub(super) fn retain_resource_tracking(
        &mut self,
        active_rule_ids: &HashSet<&str>,
        active_pids: &HashSet<u32>,
    ) {
        self.resource_since.retain(|(rule_id, pid), _| {
            active_rule_ids.contains(rule_id.as_str()) && active_pids.contains(pid)
        });
    }

    /// Since when `(rule, pid)` has been above its thresholds; inserts `now` on first sight.
    pub(super) fn resource_since(&mut self, rule_id: &str, pid: u32, now: Instant) -> Instant {
        *self
            .resource_since
            .entry((rule_id.to_string(), pid))
            .or_insert(now)
    }

    pub(super) fn reset_resource_dwell(&mut self, rule_id: &str, pid: u32, now: Instant) {
        self.resource_since
            .insert((rule_id.to_string(), pid), now);
    }

    pub(super) fn clear_resource_dwell(&mut self, rule_id: &str, pid: u32) {
        self.resource_since.remove(&(rule_id.to_string(), pid));
    }

    pub(super) fn clear_resource_tracking(&mut self) {
        self.resource_since.clear();
    }

    pub(super) fn resource_tracking_is_empty(&self) -> bool {
        self.resource_since.is_empty()
    }
}
