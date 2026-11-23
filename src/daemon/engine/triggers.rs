use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::daemon::actions::ActionRunner;
use crate::daemon::watcher::{ProcessEvent, ProcessWatcher};
use crate::models::{Action, ProcessMatchMode, Rule, RuleCondition, RuleConditionItem, ScheduleType, SystemEventType, Trigger};

use super::executions::{BoundProcess, ExecutionOrigin};
use super::scheduler::Scheduler;
use super::DaemonEngine;

impl DaemonEngine {
    pub async fn tick_process_events(&mut self) -> bool {
        self.drain_action_events();
        let events = self.watcher.poll_changes();
        let had_events = !events.is_empty();
        for event in events {
            self.handle_process_event(event).await;
        }
        had_events
    }

    /// Matches an optional device filter against the interface path reported by Windows.
    /// An empty filter accepts any device; otherwise the filter is a case-insensitive
    /// substring, which works for IDs (`VID_1234`), interface paths and drive letters.
    pub fn device_filter_matches(filter: Option<&str>, device_id: Option<&str>) -> bool {
        crate::daemon::conditions::device_filter_matches(filter, device_id)
    }

    pub fn matches_process(pattern: &str, actual: &str, mode: ProcessMatchMode) -> bool {
        crate::daemon::conditions::matches_process(pattern, actual, mode)
    }

    #[allow(dead_code)] // Facade kept for callers and tests; production paths use `conditions`.
    pub fn evaluate_condition(condition: &RuleCondition) -> bool {
        crate::daemon::conditions::evaluate_condition(condition)
    }

    pub fn evaluate_rule_conditions(conditions: &[RuleConditionItem]) -> bool {
        crate::daemon::conditions::evaluate_rule_conditions(conditions)
    }

    pub async fn handle_process_event(&mut self, event: ProcessEvent) {
        let action_str = if event.is_started { "started" } else { "stopped" };
        self.log(
            "INFO",
            "PROCESS",
            &format!("Process {} (PID: {}) {}", event.process_name, event.pid, action_str),
        );

        if event.is_started {
            // Check rules on Process Started
            let rules = self.config.rules.clone();
            for rule in &rules {
                if !rule.is_enabled {
                    continue;
                }

                if let Some(Trigger::Process { process_name, match_mode, single_instance }) = &rule.trigger {
                    if Self::matches_process(process_name, &event.process_name, *match_mode) {
                        if *single_instance {
                            let pids = self.active_rule_pids.entry(rule.id.clone()).or_default();
                            let is_first = pids.is_empty();
                            pids.insert(event.pid);

                            if is_first {
                                if !rule.actions.is_empty() {
                                    let _ = self.trigger(rule, ExecutionOrigin::Process, &event.process_name, event.pid, HashMap::new()).await;
                                }
                            } else if !rule.actions.is_empty() {
                                self.log(
                                    "INFO",
                                    "RULE",
                                    &format!(
                                        "Rule '{}' skipped (Single Instance): another instance of {} is already running (PID: {})",
                                        rule.name, event.process_name, event.pid
                                    ),
                                );
                            }
                        } else if !rule.actions.is_empty() {
                            let _ = self.trigger(rule, ExecutionOrigin::Process, &event.process_name, event.pid, HashMap::new()).await;
                        }
                    }
                }
            }
        } else {
            // Process STOPPED:
            let rules = self.config.rules.clone();
            let rules_map: HashMap<String, Rule> = rules.iter().map(|r| (r.id.clone(), r.clone())).collect();

            // Track which single_instance rules have completely terminated (all instances closed)
            let mut single_instance_rules_exited: HashSet<String> = HashSet::new();
            for rule in &rules {
                if let Some(Trigger::Process { process_name, match_mode, single_instance }) = &rule.trigger {
                    if *single_instance && Self::matches_process(process_name, &event.process_name, *match_mode) {
                        if let Some(pids) = self.active_rule_pids.get_mut(&rule.id) {
                            pids.remove(&event.pid);
                            if pids.is_empty() {
                                single_instance_rules_exited.insert(rule.id.clone());
                            }
                        }
                    }
                }
            }
            for rule_id in &single_instance_rules_exited {
                self.active_rule_pids.remove(rule_id);
            }

            // 1. Auto-close any bound processes that were launched for this process (Companion Mode in Rules)
            let to_kill: Vec<BoundProcess> = self.executions.take_bound_processes_where(|bp| {
                let is_match = bp.primary_pid == event.pid
                    || bp.primary_process_name.eq_ignore_ascii_case(&event.process_name);
                if !is_match {
                    return false;
                }
                // If the rule has single_instance enabled, only kill if all instances have exited
                if let Some(rule) = rules_map.get(&bp.rule_id) {
                    if let Some(Trigger::Process { single_instance: true, .. }) = &rule.trigger {
                        return single_instance_rules_exited.contains(&bp.rule_id);
                    }
                }
                true
            });

            for bp in to_kill {
                let (target_name, action) = Self::bound_satellite_kill(&bp.target_executable);

                self.log(
                    "ACTION",
                    "RULE",
                    &format!(
                        "Auto-closing bound process '{}' (rule: {}) because '{}' terminated",
                        target_name, bp.rule_id, event.process_name
                    ),
                );

                let ctx = HashMap::new();
                let _ = ActionRunner::execute(&action, &ctx).await;
            }

            // 1.1. Auto-close any bound overlays that were launched for this process (Companion Mode in Rules)
            let to_close_overlays = self.executions.take_bound_overlays_where(|bo| {
                let is_match = bo.primary_pid == event.pid
                    || bo.primary_process_name.eq_ignore_ascii_case(&event.process_name);
                if !is_match {
                    return false;
                }
                if let Some(rule) = rules_map.get(&bo.rule_id) {
                    if let Some(Trigger::Process { single_instance: true, .. }) = &rule.trigger {
                        return single_instance_rules_exited.contains(&bo.rule_id);
                    }
                }
                true
            });

            for bo in to_close_overlays {
                self.log(
                    "ACTION",
                    "RULE",
                    &format!(
                        "Auto-closing bound overlay '{}' (rule: {}) because '{}' terminated",
                        bo.overlay_id, bo.rule_id, event.process_name
                    ),
                );
                crate::daemon::overlay_manager::hide_overlay(&bo.overlay_id);
            }

            // 1.2. Also signal overlay_manager to auto-close any standalone overlays with close_with_target == true
            crate::daemon::overlay_manager::check_process_terminated(&event.process_name);

            // 2. Check rules with on_exit_actions
            for rule in &rules {
                if !rule.is_enabled || rule.on_exit_actions.is_empty() {
                    continue;
                }

                if let Some(Trigger::Process { process_name, match_mode, single_instance }) = &rule.trigger {
                    if Self::matches_process(process_name, &event.process_name, *match_mode) {
                        if *single_instance {
                            if single_instance_rules_exited.contains(&rule.id) {
                                let _ = self.try_execute_exit_actions(rule, &event.process_name, event.pid).await;
                            }
                        } else {
                            let _ = self.try_execute_exit_actions(rule, &event.process_name, event.pid).await;
                        }
                    }
                }
            }

            // 3. Auto-clear Discord RPC if configured on process rules
            for rule in &rules {
                if let Some(Trigger::Process { process_name, match_mode, .. }) = &rule.trigger {
                    if Self::matches_process(process_name, &event.process_name, *match_mode) {
                        let has_auto_clear = rule.actions.iter().any(|a| matches!(a, Action::DiscordRpcSet { auto_clear_on_process_exit: true, .. }));
                        if has_auto_clear {
                            let _ = crate::daemon::discord_rpc::clear_presence().await;
                        }
                    }
                }
            }
        }
    }

    pub async fn handle_system_event(&mut self, event: SystemEventType, device_id: Option<&str>) {
        self.drain_action_events();
        let event_name = format!("{:?}", event);
        self.log("INFO", "SYSTEM", &format!("System Event: {}", event_name));

        let rules = self.config.rules.clone();
        for rule in &rules {
            if !rule.is_enabled {
                continue;
            }
            if self.executions.has_recurring_test(&rule.id) {
                continue;
            }

            if let Some(Trigger::SystemEvent {
                event_type,
                device_id_filter,
                ..
            }) = &rule.trigger
            {
                if *event_type == event
                    && Self::device_filter_matches(device_id_filter.as_deref(), device_id)
                {
                    let _ = self.trigger(rule, ExecutionOrigin::SystemEvent, &event_name, 0, HashMap::new()).await;
                }
            }
        }
    }

    pub async fn tick_schedule(&mut self) {
        self.drain_action_events();
        let now_instant = Instant::now();
        let now_local = chrono::Local::now();
        let rules = self.config.rules.clone();

        for rule in &rules {
            if !rule.is_enabled {
                continue;
            }
            if self.executions.has_recurring_test(&rule.id) {
                continue;
            }

            if let Some(Trigger::Schedule {
                schedule_type,
                interval_sec,
                daily_time,
                days_of_week,
                day_of_month,
                target_datetime,
                ..
            }) = &rule.trigger
            {
                match schedule_type {
                    ScheduleType::Interval => {
                        let interval = *interval_sec;
                        if self.scheduler.interval_due(
                            rule,
                            &mut self.rule_last_triggered,
                            now_instant,
                            now_local,
                            interval,
                            target_datetime.as_deref(),
                        ) {
                            let _ = self.trigger(
                                rule,
                                ExecutionOrigin::Schedule,
                                &format!("Schedule Interval {}s", interval),
                                0,
                                HashMap::new(),
                            ).await;
                        }
                    }
                    ScheduleType::Daily => {
                        if let Some(t_str) = daily_time {
                            if self.scheduler.daily_due(
                                rule,
                                now_instant,
                                now_local,
                                t_str,
                                days_of_week,
                            ) {
                                let _ = self.trigger(
                                    rule,
                                    ExecutionOrigin::Schedule,
                                    &format!("Daily Schedule {}", t_str),
                                    0,
                                    HashMap::new(),
                                ).await;
                            }
                        }
                    }
                    ScheduleType::Monthly => {
                        if Scheduler::monthly_due(
                            rule,
                            now_local,
                            *day_of_month,
                            daily_time.as_deref(),
                        ) {
                            let target_day = day_of_month.unwrap_or(1).clamp(1, 31);
                            let target_time_str = daily_time.as_deref().unwrap_or("12:00:00");
                            let _ = self.trigger(
                                rule,
                                ExecutionOrigin::Schedule,
                                &format!("Monthly Schedule (Day {}, {})", target_day, target_time_str),
                                0,
                                HashMap::new(),
                            ).await;
                        }
                    }
                    ScheduleType::OneShot => {
                        if let Some(target) = target_datetime {
                            if Scheduler::oneshot_due(
                                rule,
                                &self.rule_last_triggered,
                                now_local,
                                target,
                            ) {
                                let started = self.trigger(
                                    rule,
                                    ExecutionOrigin::Schedule,
                                    &format!("One-Shot Schedule {}", target),
                                    0,
                                    HashMap::new(),
                                ).await;
                                // Only a run that actually started consumes the one-shot: a rule
                                // that is waiting for its conditions must stay armed.
                                if started.is_some() {
                                    let mut disabled = false;
                                    if let Some(stored) = self.config.rules.iter_mut().find(|r| r.id == rule.id) {
                                        if stored.is_enabled {
                                            stored.is_enabled = false;
                                            disabled = true;
                                        }
                                    }
                                    if disabled {
                                        self.persist_rules();
                                        self.refresh_live_rules();
                                    }
                                }
                            }
                        }
                    }
                    ScheduleType::AppStartup => {
                        // Handled on initial startup
                    }
                    _ => {}
                }
            }
        }

    }

    /// Samples the foreground window once and triggers matching `window_focus` rules. Returns
    /// whether any enabled rule follows the foreground window, so the daemon loop can back off
    /// when none does.
    #[cfg(windows)]
    pub async fn tick_window_focus(&mut self) -> bool {
        let has_focus_rules = self.config.rules.iter().any(|r| {
            r.is_enabled && matches!(r.trigger, Some(Trigger::WindowFocus { .. }))
        });
        if !has_focus_rules {
            self.last_foreground_pid = None;
            return false;
        }

        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
        let hwnd = unsafe { GetForegroundWindow() };
        let (fg_pid, is_null) = if hwnd.0.is_null() {
            (0, true)
        } else {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            (pid, pid == 0)
        };

        if is_null {
            if let Some(old_id) = self.last_foreground_pid.take() {
                let snap = ProcessWatcher::snapshot_process_map();
                if let Some(old_name) = snap.get(&old_id).cloned() {
                    let rules = self.config.rules.clone();
                    for rule in &rules {
                        if !rule.is_enabled {
                            continue;
                        }
                        if let Some(Trigger::WindowFocus {
                            process_name,
                            on_focus_lost,
                            ..
                        }) = &rule.trigger
                        {
                            if *on_focus_lost
                                && Self::matches_process(
                                    process_name,
                                    &old_name,
                                    ProcessMatchMode::Exact,
                                )
                            {
                                let _ = self
                                    .trigger(
                                        rule,
                                        ExecutionOrigin::Process,
                                        &format!("WindowFocus Lost {}", old_name),
                                        old_id,
                                        HashMap::new(),
                                    )
                                    .await;
                            }
                        }
                    }
                }
            }
            return true;
        }

        if fg_pid != 0 && self.last_foreground_pid != Some(fg_pid) {
            let old_pid = self.last_foreground_pid;
            self.last_foreground_pid = Some(fg_pid);

            let snap = ProcessWatcher::snapshot_process_map();
            let rules = self.config.rules.clone();
            if let Some(fg_name) = snap.get(&fg_pid).cloned() {
                for rule in &rules {
                    if !rule.is_enabled {
                        continue;
                    }
                    if let Some(Trigger::WindowFocus {
                        process_name,
                        on_focus_gained,
                        ..
                    }) = &rule.trigger
                    {
                        if *on_focus_gained
                            && Self::matches_process(
                                process_name,
                                &fg_name,
                                ProcessMatchMode::Exact,
                            )
                        {
                            let _ = self
                                .trigger(
                                    rule,
                                    ExecutionOrigin::Process,
                                    &format!("WindowFocus Gained {}", fg_name),
                                    fg_pid,
                                    HashMap::new(),
                                )
                                .await;
                        }
                    }
                }
            }

            if let Some(old_id) = old_pid {
                if let Some(old_name) = snap.get(&old_id).cloned() {
                    for rule in &rules {
                        if !rule.is_enabled {
                            continue;
                        }
                        if let Some(Trigger::WindowFocus {
                            process_name,
                            on_focus_lost,
                            ..
                        }) = &rule.trigger
                        {
                            if *on_focus_lost
                                && Self::matches_process(
                                    process_name,
                                    &old_name,
                                    ProcessMatchMode::Exact,
                                )
                            {
                                let _ = self
                                    .trigger(
                                        rule,
                                        ExecutionOrigin::Process,
                                        &format!("WindowFocus Lost {}", old_name),
                                        old_id,
                                        HashMap::new(),
                                    )
                                    .await;
                            }
                        }
                    }
                }
            }
        }

        true
    }

    pub async fn tick_resource_threshold(&mut self) {
        let rules = self.config.rules.clone();
        let has_resource_rules = rules.iter().any(|r| r.is_enabled && matches!(r.trigger, Some(Trigger::ResourceThreshold { .. })));
        if !has_resource_rules {
            if !self.scheduler.resource_tracking_is_empty() {
                self.scheduler.clear_resource_tracking();
            }
            return;
        }

        let snap = ProcessWatcher::get_detailed_snapshot();
        let active_pids: HashSet<u32> = snap.iter().map(|p| p.pid).collect();
        let active_rule_ids: HashSet<&str> = rules.iter().filter(|r| r.is_enabled).map(|r| r.id.as_str()).collect();
        self.scheduler.retain_resource_tracking(&active_rule_ids, &active_pids);

        for rule in &rules {
            if !rule.is_enabled {
                continue;
            }
            if let Some(Trigger::ResourceThreshold { process_name, ram_mb, cpu_percent, duration_sec }) = &rule.trigger {
                for proc in &snap {
                    if Self::matches_process(process_name, &proc.name, ProcessMatchMode::Exact) {
                        let mem_mb = proc.memory_bytes / (1024 * 1024);
                        // Mirror `evaluate_condition`: every configured limit must hold, and a
                        // CPU-only rule fires on CPU alone.
                        let over_mem = ram_mb.is_none_or(|limit| mem_mb >= limit);
                        let over_cpu = cpu_percent.is_none_or(|limit| proc.cpu_percent >= limit);
                        let over_threshold =
                            over_mem && over_cpu && (ram_mb.is_some() || cpu_percent.is_some());

                        if over_threshold {
                            let now = Instant::now();
                            let start = self.scheduler.resource_since(&rule.id, proc.pid, now);
                            if start.elapsed().as_secs() >= *duration_sec {
                                self.scheduler.reset_resource_dwell(&rule.id, proc.pid, now);
                                let detail = match (ram_mb, cpu_percent) {
                                    (Some(_), Some(_)) => format!("{} MB, {:.0}% CPU", mem_mb, proc.cpu_percent),
                                    (Some(_), None) => format!("{} MB", mem_mb),
                                    (None, Some(_)) => format!("{:.0}% CPU", proc.cpu_percent),
                                    (None, None) => String::new(),
                                };
                                let _ = self.trigger(
                                    rule,
                                    ExecutionOrigin::SystemEvent,
                                    &format!("Resource Threshold: {} ({})", proc.name, detail),
                                    proc.pid,
                                    HashMap::new(),
                                ).await;
                            }
                        } else {
                            self.scheduler.clear_resource_dwell(&rule.id, proc.pid);
                        }
                    }
                }
            }
        }
    }
}
