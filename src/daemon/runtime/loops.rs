use crate::daemon::engine::{DaemonEngine, ExecutionOrigin};
use crate::daemon::hotkeys;
use crate::daemon::system_events::SystemEventsWatcher;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(super) async fn spawn_hotkey_loop(
    engine_hotkey: Arc<Mutex<DaemonEngine>>,
    mut hotkey_rx: tokio::sync::mpsc::UnboundedReceiver<hotkeys::HotkeyEvent>,
    hold_tasks_loop: Arc<
        tokio::sync::Mutex<std::collections::HashMap<String, tokio::task::AbortHandle>>,
    >,
) {
    while let Some(ev) = hotkey_rx.recv().await {
        let mut eng = engine_hotkey.lock().await;
        match ev {
            hotkeys::HotkeyEvent::Trigger(rule_id) => {
                let was_looping = if let Some(h) = hold_tasks_loop.lock().await.remove(&rule_id)
                {
                    h.abort();
                    true
                } else {
                    false
                };

                if eng.is_rule_executing(&rule_id) || was_looping {
                    eng.stop_rule_execution(&rule_id);
                    crate::daemon::actions::release_all_simulated_inputs();
                    eng.mark_rule_triggered(&rule_id);
                } else if let Some(rule) =
                    eng.config.rules.iter().find(|r| r.id == rule_id).cloned()
                {
                    if rule.is_enabled {
                        let is_repeat = matches!(
                            rule.trigger,
                            Some(crate::models::Trigger::GlobalHotkey {
                                repeat_while_held: true,
                                ..
                            })
                        );
                        if is_repeat {
                            let engine_for_loop = Arc::clone(&engine_hotkey);
                            let rule_id_clone = rule_id.clone();
                            let hold_tasks_inner = Arc::clone(&hold_tasks_loop);

                            let task = tokio::spawn(async move {
                                loop {
                                    let handle_opt = {
                                        let mut eng = engine_for_loop.lock().await;
                                        if let Some(r) = eng
                                            .config
                                            .rules
                                            .iter()
                                            .find(|r| r.id == rule_id_clone)
                                            .cloned()
                                        {
                                            if !r.is_enabled {
                                                break;
                                            }
                                            eng.trigger(&r, ExecutionOrigin::Hotkey, "Global Hotkey (Loop)", 0, HashMap::new())
                                                .await
                                        } else {
                                            break;
                                        }
                                    };

                                    if let Some(handle) = handle_opt {
                                        match handle.await {
                                            Ok(()) => {}
                                            Err(_) => {
                                                break;
                                            }
                                        }
                                    } else {
                                        break;
                                    }
                                }
                                hold_tasks_inner.lock().await.remove(&rule_id_clone);
                            });
                            hold_tasks_loop
                                .lock()
                                .await
                                .insert(rule_id, task.abort_handle());
                        } else {
                            let _ = eng.trigger(&rule, ExecutionOrigin::Hotkey, "Global Hotkey", 0, HashMap::new()).await;
                        }
                    }
                }
            }
            hotkeys::HotkeyEvent::HoldStart(rule_id) => {
                if let Some(h) = hold_tasks_loop.lock().await.remove(&rule_id) {
                    h.abort();
                }

                if !eng.is_rule_executing(&rule_id) {
                    if let Some(rule) =
                        eng.config.rules.iter().find(|r| r.id == rule_id).cloned()
                    {
                        if rule.is_enabled {
                            let engine_for_hold = Arc::clone(&engine_hotkey);
                            let rule_id_clone = rule_id.clone();
                            let hold_tasks_inner = Arc::clone(&hold_tasks_loop);

                            let task = tokio::spawn(async move {
                                loop {
                                    if !crate::daemon::hotkeys::is_hotkey_held(&rule_id_clone) {
                                        break;
                                    }

                                    let handle_opt = {
                                        let mut eng = engine_for_hold.lock().await;
                                        if let Some(r) = eng
                                            .config
                                            .rules
                                            .iter()
                                            .find(|r| r.id == rule_id_clone)
                                            .cloned()
                                        {
                                            if !r.is_enabled {
                                                break;
                                            }
                                            eng.trigger(&r, ExecutionOrigin::Hotkey, "Global Hotkey (Hold)", 0, HashMap::new())
                                                .await
                                        } else {
                                            break;
                                        }
                                    };

                                    if let Some(handle) = handle_opt {
                                        match handle.await {
                                            Ok(()) => {}
                                            Err(_) => {
                                                // Rule execution task was cancelled / aborted; stop loop immediately!
                                                break;
                                            }
                                        }
                                    } else {
                                        break;
                                    }

                                    if !crate::daemon::hotkeys::is_hotkey_repeat_held(
                                        &rule_id_clone,
                                    ) {
                                        break;
                                    }

                                    if !crate::daemon::hotkeys::is_hotkey_held(&rule_id_clone) {
                                        break;
                                    }

                                    // Yield brief pause between loop cycles so input queue & events can breathe
                                    tokio::time::sleep(std::time::Duration::from_millis(25))
                                        .await;
                                }

                                let mut tasks = hold_tasks_inner.lock().await;
                                tasks.remove(&rule_id_clone);
                            });

                            hold_tasks_loop
                                .lock()
                                .await
                                .insert(rule_id.clone(), task.abort_handle());
                        }
                    }
                }
            }
            hotkeys::HotkeyEvent::HoldStop(rule_id) => {
                if let Some(h) = hold_tasks_loop.lock().await.remove(&rule_id) {
                    h.abort();
                }
                // Releasing the hotkey aborts the run; it must not disable the rule.
                eng.abort_rule_executions(&rule_id);
                crate::daemon::actions::release_all_simulated_inputs();
            }
        }
    }
}

pub(super) async fn spawn_runtime_flush_loop(engine_runtime: Arc<Mutex<DaemonEngine>>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(1000));
    loop {
        interval.tick().await;
        // One CPU/RAM sample per second feeds the analytics dashboard's live chart. It reads only
        // the two counters the chart needs — not the full telemetry snapshot, which also touches
        // the battery and the COM audio endpoint — and it is taken outside the engine lock, so the
        // Win32 calls never stall a busy engine. The same tick samples the daemon's own CPU,
        // memory and disk use for the SYSTEM LOAD card.
        let metrics = crate::daemon::telemetry::get_cpu_ram_metrics();
        let daemon_usage = crate::daemon::telemetry::sample_daemon_usage();
        let sampled_at = std::time::Instant::now();
        let mut eng = engine_runtime.lock().await;
        eng.persist_runtime_if_due();
        eng.persist_overlay_positions_if_due();
        if let Some((cpu, ram)) = metrics {
            eng.record_analytics_sample(cpu, ram, sampled_at);
        }
        eng.record_daemon_usage(daemon_usage);
    }
}

/// Whether the process loop must keep polling: any enabled process or resource rule needs its
/// lifecycle diff, and a live bound companion needs its exit noticed.
fn needs_process_polling(
    rules: &[crate::models::Rule],
    has_active_bound_companions: bool,
) -> bool {
    has_active_bound_companions
        || rules.iter().any(|rule| {
            rule.is_enabled
                && matches!(
                    rule.trigger,
                    Some(
                        crate::models::Trigger::Process { .. }
                            | crate::models::Trigger::ResourceThreshold { .. }
                    )
                )
        })
}

/// Process-loop delay. A dormant loop ignores event churn — unrelated processes starting and
/// exiting must not spin a daemon that has nothing to match them against. ETW drains a push queue
/// and can stay moderately responsive; snapshot polling is the expensive path, so an active rule
/// only gets the fast tick when its own events need draining.
fn process_poll_delay_ms(etw_active: bool, needs_polling: bool, had_events: bool) -> u64 {
    if !needs_polling {
        return if etw_active { 500 } else { 3000 };
    }
    if etw_active {
        50
    } else if had_events {
        150
    } else {
        300
    }
}

pub(super) async fn spawn_process_loop(engine_process: Arc<Mutex<DaemonEngine>>) {
    loop {
        let delay_ms = {
            let mut eng = engine_process.lock().await;
            let had_events = eng.tick_process_events().await;
            let needs_polling = needs_process_polling(
                &eng.config.rules,
                eng.has_active_bound_companions(),
            );
            process_poll_delay_ms(eng.watcher().is_etw_active(), needs_polling, had_events)
        };
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
    }
}

pub(super) async fn spawn_window_loop(engine_window: Arc<Mutex<DaemonEngine>>) {
    loop {
        let has_focus_rules = {
            let mut eng = engine_window.lock().await;
            eng.tick_window_focus().await
        };
        // Deep sleep when no rule follows the foreground window; the tick still clears the
        // tracked pid so a rule added later starts from a clean baseline.
        let delay_ms = if has_focus_rules { 150 } else { 3000 };
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
    }
}

/// Whether any enabled rule reads the state the system-event loop samples: a `system` trigger or
/// an `event` condition in the preconditions or an inline If/Else branch.
fn has_system_event_consumers(rules: &[crate::models::Rule]) -> bool {
    rules.iter().filter(|rule| rule.is_enabled).any(|rule| {
        matches!(rule.trigger, Some(crate::models::Trigger::SystemEvent { .. }))
            || rule.conditions.iter().any(is_event_condition)
            || {
                let mut found = false;
                let mut visit = |action: &crate::models::Action| {
                    if let crate::models::Action::IfElse {
                        conditions,
                        else_if_branches,
                        ..
                    } = action
                    {
                        if conditions.iter().any(is_event_condition)
                            || else_if_branches
                                .iter()
                                .any(|branch| branch.conditions.iter().any(is_event_condition))
                        {
                            found = true;
                        }
                    }
                };
                rule.visit_actions(&mut visit);
                found
            }
    })
}

fn is_event_condition(condition: &crate::models::RuleConditionItem) -> bool {
    matches!(
        condition.condition,
        crate::models::RuleCondition::Event { .. }
    )
}

pub(super) async fn spawn_system_loop(engine_sys: Arc<Mutex<DaemonEngine>>) {
    let mut sys_watcher = SystemEventsWatcher::new();
    let notification = crate::daemon::system_events::notification_handle();
    loop {
        let delay = {
            let eng = engine_sys.lock().await;
            if has_system_event_consumers(&eng.config.rules) {
                tokio::time::Duration::from_millis(2000)
            } else {
                // Nothing reads event state: keep the polling sources asleep. Suspend, display
                // and device broadcasts still wake the loop through the notification window.
                tokio::time::Duration::from_secs(10)
            }
        };
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = notification.notified() => {}
        }
        let events = sys_watcher.check_events();
        if !events.is_empty() {
            let mut eng = engine_sys.lock().await;
            for ev in events {
                eng.handle_system_event(ev.event_type, ev.device_id.as_deref())
                    .await;
            }
        }
    }
}

pub(super) async fn spawn_schedule_loop(engine_sched: Arc<Mutex<DaemonEngine>>) {
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    {
        let mut eng = engine_sched.lock().await;
        let rules = eng.config.rules.clone();
        for rule in &rules {
            if rule.is_enabled {
                if let Some(crate::models::Trigger::Schedule {
                    schedule_type: crate::models::ScheduleType::AppStartup,
                    ..
                }) = &rule.trigger
                {
                    let _ = eng.trigger(rule, ExecutionOrigin::Schedule, "App Startup", 0, HashMap::new()).await;
                }
            }
        }
    }

    loop {
        let delay_ms = {
            let mut eng = engine_sched.lock().await;
            let has_schedules = eng.config.rules.iter().any(|r| {
                r.is_enabled
                    && matches!(r.trigger, Some(crate::models::Trigger::Schedule { .. }))
            });
            let has_resource_rules = eng.config.rules.iter().any(|r| {
                r.is_enabled
                    && matches!(
                        r.trigger,
                        Some(crate::models::Trigger::ResourceThreshold { .. })
                    )
            });
            if has_schedules {
                eng.tick_schedule().await;
            }
            if has_resource_rules {
                eng.tick_resource_threshold().await;
            }
            if has_schedules || has_resource_rules {
                1000
            } else {
                5000 // Dormant mode: 5s interval when no schedule or resource rules exist
            }
        };
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
    }
}

pub(super) async fn spawn_webhook_loop(engine_webhook: Arc<Mutex<DaemonEngine>>) {
    // The local HTTP server is only useful while an enabled webhook rule exists. Keep it parked
    // otherwise instead of holding a listening socket; the rule snapshot is re-read so a rule
    // saved or disabled through the IPC surface starts and stops the server without a restart.
    let mut running: Option<(
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    )> = None;

    loop {
        let needed = {
            let eng = engine_webhook.lock().await;
            crate::daemon::webhook_server::has_enabled_webhook_rules(&eng.config.rules)
        };

        match (needed, running.is_some()) {
            (true, false) => {
                let server = crate::daemon::webhook_server::WebhookServer::new(
                    Arc::clone(&engine_webhook),
                    crate::daemon::webhook_server::DEFAULT_WEBHOOK_PORT,
                );
                let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
                crate::logging::info(
                    "WEBHOOK",
                    "Enabled webhook rule(s) found; starting the local webhook server",
                );
                let handle = tokio::spawn(async move {
                    server.run(stop_rx).await;
                });
                running = Some((stop_tx, handle));
            }
            (false, true) => {
                if let Some((stop_tx, handle)) = running.take() {
                    let _ = stop_tx.send(());
                    let _ = handle.await;
                }
            }
            _ => {}
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
    }
}

pub(super) async fn spawn_file_loop(engine_file: Arc<Mutex<DaemonEngine>>) {
    let mut file_watcher = crate::daemon::file_watcher::FileWatcherService::new(engine_file);
    loop {
        let delay_ms = file_watcher.poll_once().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
    }
}

pub(super) async fn spawn_update_check(
    engine_update: Arc<Mutex<DaemonEngine>>,
    is_daemon_cli: bool,
    auto_install: bool,
) {
    tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
    let current_ver = env!("CARGO_PKG_VERSION").to_string();
    match crate::updater::check_github_release(crate::updater::GITHUB_REPO, &current_ver).await {
        Ok(Some(info)) => {
            crate::logging::debug(
                "DAEMON",
                &format!(
                    "Update available: v{} (auto-install: {auto_install})",
                    info.version
                ),
            );
            {
                let mut eng = engine_update.lock().await;
                eng.set_available_update(Some(info.clone()));
            }
            crate::daemon::tray::set_available_update(Some(info.clone()));

            if auto_install {
                // Silent automatic update: no notification overlay, no progress overlay
                crate::daemon::tray::trigger_update_flow(true);
            } else if is_daemon_cli && !crate::single_instance::is_gui_running() {
                crate::daemon::overlay_manager::show_update_notification(&info.version);
            }
        }
        Ok(None) => {
            crate::logging::debug(
                "DAEMON",
                &format!("Synector is up to date (v{current_ver})"),
            );
        }
        Err(err) => {
            crate::logging::warn("DAEMON", &format!("Update check failed: {err}"));
            eprintln!("[DAEMON] Update check failed: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        Action, ElseIfBranch, Rule, RuleCondition, RuleConditionItem, SystemEventType, Trigger,
    };

    fn system_event_rule() -> Rule {
        Rule {
            trigger: Some(Trigger::SystemEvent {
                event_type: SystemEventType::SessionLock,
                device_id_filter: None,
            }),
            ..Default::default()
        }
    }

    fn process_rule() -> Rule {
        Rule {
            trigger: Some(Trigger::Process {
                process_name: "game.exe".to_string(),
                match_mode: Default::default(),
                single_instance: false,
                event: Default::default(),
            }),
            ..Default::default()
        }
    }

    fn event_condition() -> RuleConditionItem {
        RuleConditionItem::new(RuleCondition::Event {
            event_type: SystemEventType::UserIdle,
            idle_duration_sec: 300,
        })
    }

    fn if_else(
        conditions: Vec<RuleConditionItem>,
        branches: Vec<ElseIfBranch>,
        else_actions: Vec<Action>,
    ) -> Action {
        Action::IfElse {
            conditions,
            then_actions: Vec::new(),
            else_if_branches: branches,
            else_actions,
        }
    }

    #[test]
    fn process_loop_sleeps_when_no_rule_or_companion_needs_polling() {
        assert!(!needs_process_polling(&[], false));
        assert!(
            needs_process_polling(&[process_rule()], false),
            "an enabled process rule needs its lifecycle diff"
        );
        assert!(
            needs_process_polling(&[], true),
            "a live bound companion needs its exit noticed"
        );

        let mut disabled = process_rule();
        disabled.is_enabled = false;
        assert!(!needs_process_polling(&[disabled], false));

        assert_eq!(process_poll_delay_ms(false, false, false), 3000);
        assert_eq!(
            process_poll_delay_ms(false, false, true),
            3000,
            "events from unrelated processes must not speed up a dormant loop"
        );
        assert_eq!(process_poll_delay_ms(false, true, false), 300);
        assert_eq!(process_poll_delay_ms(false, true, true), 150);
        assert_eq!(process_poll_delay_ms(true, false, false), 500);
        assert_eq!(process_poll_delay_ms(true, true, false), 50);
    }

    #[test]
    fn system_loop_stays_dormant_without_system_event_consumers() {
        assert!(!has_system_event_consumers(&[]));
        assert!(!has_system_event_consumers(&[Rule::default()]));

        let mut disabled = system_event_rule();
        disabled.is_enabled = false;
        assert!(
            !has_system_event_consumers(&[disabled]),
            "a disabled system rule must not keep the system-event loop polling"
        );
    }

    #[test]
    fn system_loop_runs_for_system_triggers_and_event_conditions() {
        assert!(has_system_event_consumers(&[system_event_rule()]));

        let precondition = Rule {
            conditions: vec![event_condition()],
            ..Default::default()
        };
        assert!(has_system_event_consumers(&[precondition]));

        let nested = Rule {
            actions: vec![if_else(
                Vec::new(),
                Vec::new(),
                vec![if_else(vec![event_condition()], Vec::new(), Vec::new())],
            )],
            ..Default::default()
        };
        assert!(
            has_system_event_consumers(&[nested]),
            "an event condition nested in an else branch still reads the sampled state"
        );

        let branch = Rule {
            actions: vec![if_else(
                Vec::new(),
                vec![ElseIfBranch {
                    conditions: vec![event_condition()],
                    actions: Vec::new(),
                }],
                Vec::new(),
            )],
            ..Default::default()
        };
        assert!(has_system_event_consumers(&[branch]));
    }
}
