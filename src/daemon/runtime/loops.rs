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
                    eng.rule_last_triggered
                        .insert(rule_id.clone(), std::time::Instant::now());
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
        let mut eng = engine_runtime.lock().await;
        eng.persist_runtime_if_due();
        eng.persist_overlay_positions_if_due();
    }
}

pub(super) async fn spawn_process_loop(engine_process: Arc<Mutex<DaemonEngine>>) {
    loop {
        let delay_ms = {
            let mut eng = engine_process.lock().await;
            let had_events = eng.tick_process_events().await;
            let has_proc_rules = eng.config.rules.iter().any(|r| {
                r.is_enabled
                    && matches!(
                        r.trigger,
                        Some(
                            crate::models::Trigger::Process { .. }
                                | crate::models::Trigger::ResourceThreshold { .. }
                        )
                    )
            }) || !eng.has_active_bound_companions();

            if eng.watcher.is_etw_active() {
                if has_proc_rules {
                    50
                } else {
                    500
                }
            } else if had_events {
                150
            } else if has_proc_rules {
                300
            } else {
                3000 // Deep sleep when no process rules exist
            }
        };
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
    }
}

pub(super) async fn spawn_window_loop(engine_window: Arc<Mutex<DaemonEngine>>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(150));
    loop {
        interval.tick().await;
        let mut eng = engine_window.lock().await;
        eng.tick_window_focus().await;
    }
}

pub(super) async fn spawn_system_loop(engine_sys: Arc<Mutex<DaemonEngine>>) {
    let mut sys_watcher = SystemEventsWatcher::new();
    let notification = crate::daemon::system_events::notification_handle();
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(2000));
    loop {
        tokio::select! {
            _ = interval.tick() => {}
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
    let server = crate::daemon::webhook_server::WebhookServer::new(
        engine_webhook,
        crate::daemon::webhook_server::DEFAULT_WEBHOOK_PORT,
    );
    server.run().await;
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
