pub mod actions;
pub mod discord_rpc;
pub mod engine;
pub mod file_watcher;
pub mod hotkeys;
pub mod ipc_server;
pub mod lua_runner;
pub mod overlay_manager;
pub mod pe_mask;
pub mod rule_activity;
pub mod rule_call;
pub mod system_events;
pub mod telemetry;
pub mod tray;
pub mod watcher;
pub mod webhook_server;

use crate::config::AppConfig;
use crate::daemon::engine::DaemonEngine;
use crate::daemon::ipc_server::IpcServer;
use crate::daemon::system_events::SystemEventsWatcher;
use crate::daemon::tray::TrayManager;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex as StdMutex};
use tokio::sync::Mutex;

/// Coordinates a daemon shutdown request with the main thread.
///
/// When the tray cannot be created the daemon runs headless, so there is no message loop that can
/// end and no place to fall through to the shutdown sequence. The main thread waits on this
/// signal instead of parking forever; `request_shutdown()` makes it proceed to On App Close rules
/// and the final runtime/overlay persistence.
struct ShutdownSignal {
    requested: StdMutex<bool>,
    signal: Condvar,
}

impl ShutdownSignal {
    const fn new() -> Self {
        Self {
            requested: StdMutex::new(false),
            signal: Condvar::new(),
        }
    }

    fn request(&self) {
        let mut requested = self
            .requested
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *requested = true;
        self.signal.notify_all();
    }

    fn wait(&self) {
        let mut requested = self
            .requested
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !*requested {
            requested = self
                .signal
                .wait(requested)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

static DAEMON_SHUTDOWN: ShutdownSignal = ShutdownSignal::new();

/// Requests a coordinated daemon shutdown. Safe to call from any thread and more than once; the
/// main thread then runs the On App Close rules and final persistence before the process exits.
pub fn request_shutdown() {
    DAEMON_SHUTDOWN.request();
    // The tray message loop blocks in `GetMessageW` and only re-checks its run flag after a
    // message, so a shutdown requested from another thread (the IPC `ShutdownDaemon` handler) has
    // to stop the flag and wake the loop. Hiding the icon posts that wake-up message and is a
    // no-op for a process that is already exiting; in headless mode there is no window at all.
    crate::daemon::tray::RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    crate::daemon::tray::set_tray_visible(false);
}

fn wait_for_shutdown_request() {
    DAEMON_SHUTDOWN.wait();
}

/// Executable of the GUI that spawned this daemon (`--gui-exe <path>`), when it reported one.
///
/// The updater replaces this executable for a portable installation with process masking: the
/// daemon then runs from the masked copy in `%LOCALAPPDATA%\Synector\bin`, which is only a cache,
/// and the GUI window may already be closed when the update starts.
static LAUNCHING_GUI_EXE: StdMutex<Option<PathBuf>> = StdMutex::new(None);

/// Parses the `--gui-exe <path>` argument. An absent flag or an empty value means "unknown" and
/// leaves the updater with its GUI-window lookup.
pub(crate) fn parse_gui_exe_argument(args: &[String]) -> Option<PathBuf> {
    let position = args.iter().position(|arg| arg == "--gui-exe")?;
    let path = PathBuf::from(args.get(position + 1)?);
    (!path.as_os_str().is_empty()).then_some(path)
}

/// Records the launching GUI executable once, at daemon start-up.
fn record_launching_gui_exe(args: &[String]) {
    let Some(path) = parse_gui_exe_argument(args) else {
        return;
    };
    let mut lock = LAUNCHING_GUI_EXE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *lock = Some(path);
}

/// The executable the updater must replace when this daemon runs as the masked copy.
///
/// The path is recorded as the GUI passed it; the consumer validates that it still exists before
/// replacing it. `None` when the daemon was not spawned by the GUI (`--daemon` directly or by
/// autostart).
pub fn launching_gui_exe() -> Option<PathBuf> {
    LAUNCHING_GUI_EXE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Runs the post-loop graceful shutdown steps in their fixed order: On App Close rules stop the
/// engine, then the overlays close synchronously (so their fade-out plays instead of the
/// kill-on-close Job Object terminating them), and only then the coalesced runtime counters and
/// overlay positions are flushed. Kept as one sequence so the order cannot drift.
fn run_shutdown_sequence(
    stop_engine: impl FnOnce(),
    hide_overlays: impl FnOnce(),
    flush_state: impl FnOnce(),
) {
    stop_engine();
    hide_overlays();
    flush_state();
}

pub fn run_daemon() {
    let args: Vec<String> = std::env::args().collect();
    record_launching_gui_exe(&args);
    // Recorded once here; the updater targets this executable for a portable update while this
    // daemon runs as the masked copy, even after the GUI window is gone.
    if let Some(gui_exe) = launching_gui_exe() {
        crate::logging::debug(
            "DAEMON",
            &format!("Launched by GUI executable: {}", gui_exe.display()),
        );
    }

    let _instance_guard = match crate::single_instance::SingleInstanceGuard::acquire_daemon() {
        Some(guard) => guard,
        None => {
            crate::logging::warn(
                "DAEMON",
                "Another instance of the Synector daemon is already running",
            );
            eprintln!("[DAEMON] Another instance of Synector daemon is already running.");
            return;
        }
    };

    crate::logging::info(
        "DAEMON",
        &format!(
            "Daemon starting: version {}, pid {}, log level {}, log file {}",
            env!("CARGO_PKG_VERSION"),
            std::process::id(),
            crate::logging::level().as_str(),
            crate::logging::current_log_path().display()
        ),
    );
    println!("[DAEMON] Initializing Synector background daemon...");

    // Pre-initialize overlay manager and install bundled fonts
    overlay_manager::init();

    let config = AppConfig::load();

    // Reconcile state that can outlive an update or an installation: a stale masked daemon copy
    // and an autostart task pointing at an old portable location.
    if config.mask_daemon_process {
        if let Err(err) = pe_mask::prepare_masked_daemon() {
            // The daemon keeps running unmasked; make the fallback visible instead of silently
            // appearing as a successful masked deployment.
            crate::logging::error(
                "DAEMON",
                &format!("Could not prepare the masked daemon image; running unmasked: {err}"),
            );
        }
    } else if !pe_mask::running_from_masked_copy() {
        // Masking was disabled while a masked copy from an earlier run is still deployed. This
        // daemon is not that copy, so the stale image can be removed here.
        pe_mask::cleanup_masked_daemon();
    } else {
        crate::logging::debug(
            "DAEMON",
            "Masking is disabled but this daemon is the masked copy; keeping the running image",
        );
    }
    crate::autorun::ensure_autorun_target_current(config.mask_daemon_process);

    let engine = Arc::new(Mutex::new(DaemonEngine::new(config.clone())));

    // Create multi-threaded Tokio runtime for background tasks
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            crate::logging::error(
                "DAEMON",
                &format!("Failed to create the Tokio runtime: {err}"),
            );
            return;
        }
    };

    // Rule-to-rule control (`rule_control` actions) asks the daemon to start or abort another
    // rule. Installed before any trigger loop can fire an action.
    let (rule_control_tx, mut rule_control_rx) =
        tokio::sync::mpsc::unbounded_channel::<rule_call::RuleControlRequest>();
    rule_call::install(rule_control_tx);
    let engine_control = Arc::clone(&engine);
    rt.spawn(async move {
        while let Some(request) = rule_control_rx.recv().await {
            let engine = Arc::clone(&engine_control);
            tokio::spawn(async move {
                rule_call::dispatch(engine, request).await;
            });
        }
    });

    // Initialize Global Hotkey Listener
    let (hotkey_tx, mut hotkey_rx) = tokio::sync::mpsc::unbounded_channel::<hotkeys::HotkeyEvent>();
    let hotkey_handle = hotkeys::start_hotkey_listener(hotkey_tx);
    hotkey_handle.reload(&config.rules);

    {
        let engine_clone = Arc::clone(&engine);
        let h_handle = hotkey_handle.clone();
        rt.block_on(async {
            let mut eng = engine_clone.lock().await;
            eng.hotkey_handle = Some(h_handle);
        });
    }

    // Spawn Global Hotkey Event Loop (supports Normal toggle, Hold mode, and Infinite macro cancellation)
    let engine_hotkey = Arc::clone(&engine);
    let hold_tasks: Arc<
        tokio::sync::Mutex<std::collections::HashMap<String, tokio::task::AbortHandle>>,
    > = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

    let hold_tasks_loop = Arc::clone(&hold_tasks);
    rt.spawn(async move {
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
                                                eng.try_execute_rule(&r, "Global Hotkey (Loop)", 0)
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
                                let _ = eng.try_execute_rule(&rule, "Global Hotkey", 0).await;
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
                                                eng.try_execute_rule(&r, "Global Hotkey (Hold)", 0)
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
    });

    // Log daemon initialization status
    {
        let autorun_status = if crate::autorun::is_autorun_enabled(crate::autorun::APP_NAME) {
            "enabled"
        } else {
            "disabled"
        };
        let engine_clone = Arc::clone(&engine);
        rt.block_on(async {
            let mut eng = engine_clone.lock().await;
            let mode = eng.watcher.get_mode_name();
            eng.log(
                "INFO",
                "DAEMON",
                &format!(
                    "Daemon initialized (autorun: {}, watcher: {})",
                    autorun_status, mode
                ),
            );
        });
    }

    // Spawn IPC Server
    let ipc_server = IpcServer::new(Arc::clone(&engine));
    rt.spawn(async move {
        ipc_server.run().await;
    });

    // Flush coalesced runtime counters (execution counts, last-run timestamps) and
    // overlay drag positions so hot paths write at a bounded rate instead of per event.
    let engine_runtime = Arc::clone(&engine);
    rt.spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(1000));
        loop {
            interval.tick().await;
            let mut eng = engine_runtime.lock().await;
            eng.persist_runtime_if_due();
            eng.persist_overlay_positions_if_due();
        }
    });

    // Spawn Process Watcher Loop (Hybrid: ETW 50ms queue drain under admin; Adaptive 150-300ms snapshot polling; Dormant when no rules)
    let engine_process = Arc::clone(&engine);
    rt.spawn(async move {
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
    });

    // Spawn Window Focus Watcher Loop (Independent regular polling cycle 150ms)
    let engine_window = Arc::clone(&engine);
    rt.spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(150));
        loop {
            interval.tick().await;
            let mut eng = engine_window.lock().await;
            eng.tick_window_focus().await;
        }
    });

    // Spawn System Events Watcher Loop. Polled sources run on a timer; the notification
    // window wakes the loop immediately for suspend/resume, display and device events.
    let engine_sys = Arc::clone(&engine);
    rt.spawn(async move {
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
    });

    // Spawn Schedule & Resource Watcher Loop (Dormant when no schedule or resource rules exist)
    let engine_sched = Arc::clone(&engine);
    rt.spawn(async move {
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
                        let _ = eng.try_execute_rule(rule, "App Startup", 0).await;
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
    });

    // Spawn Webhook Server Loop
    let engine_webhook = Arc::clone(&engine);
    rt.spawn(async move {
        let server = crate::daemon::webhook_server::WebhookServer::new(
            engine_webhook,
            crate::daemon::webhook_server::DEFAULT_WEBHOOK_PORT,
        );
        server.run().await;
    });

    // Spawn File Watcher Loop (Dormant when no file watcher rules exist)
    let engine_file = Arc::clone(&engine);
    rt.spawn(async move {
        let mut file_watcher = crate::daemon::file_watcher::FileWatcherService::new(engine_file);
        loop {
            let delay_ms = file_watcher.poll_once().await;
            tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
        }
    });

    // Background update check on daemon startup
    if config.check_updates_on_startup {
        let engine_update = Arc::clone(&engine);
        let is_daemon_cli = std::env::args().any(|a| a == "--daemon" || a == "-d" || a == "--service" || a == "-s")
            && !std::env::args().any(|a| a == "--spawned-by-gui");
        let auto_install = config.auto_install_updates;

        rt.spawn(async move {
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
                        eng.available_update = Some(info.clone());
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
        });
    }

    crate::logging::info(
        "DAEMON",
        "Service started, listening on the Synector named pipe",
    );
    println!("[DAEMON] Service started. 0.0% CPU, listening on Named Pipe.");

    // Run native Win32 Tray on main thread
    match TrayManager::new(
        config.tray_icon_path.clone(),
        config.hide_tray_icon,
        config.mask_daemon_process,
        Arc::clone(&engine),
        rt.handle().clone(),
    ) {
        Ok(tray) => {
            tray.run_message_loop();
            // The tray message loop ends only on an explicit shutdown request.
            request_shutdown();
        }
        Err(err) => {
            crate::logging::error(
                "DAEMON",
                &format!("Tray initialization failed: {err}; running headless"),
            );
            eprintln!("[DAEMON] Tray initialization error: {err}. Running headless loop.");
            // Block until something asks the daemon to stop (for example the IPC ShutdownDaemon
            // handler), then fall through to the shared shutdown sequence below instead of
            // parking forever with no way to persist the final state.
            wait_for_shutdown_request();
        }
    }

    crate::logging::info("DAEMON", "Daemon shutting down");

    // Close GUI if daemon exits
    if crate::single_instance::is_gui_running() {
        crate::single_instance::close_existing_gui_window();
    }

    // One graceful sequence for both the tray Exit path and the headless shutdown request:
    // On App Close rules run first, then the overlays close synchronously (the kill-on-close Job
    // Object would otherwise terminate them before their fade-out plays), and the coalesced
    // runtime counters / overlay positions are flushed last.
    run_shutdown_sequence(
        || {
            rt.block_on(async {
                let mut eng = engine.lock().await;
                eng.execute_shutdown_rules().await;
            });
        },
        overlay_manager::hide_all_sync,
        || {
            rt.block_on(async {
                let mut eng = engine.lock().await;
                eng.persist_overlay_positions_now();
                eng.persist_runtime_now();
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shutdown_signal_wakes_waiter() {
        let signal = Arc::new(ShutdownSignal::new());
        let waiter = {
            let signal = Arc::clone(&signal);
            std::thread::spawn(move || signal.wait())
        };
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(
            !waiter.is_finished(),
            "the headless wait must block until a shutdown is requested"
        );
        signal.request();
        waiter.join().expect("waiter must finish after the shutdown request");
    }

    #[test]
    fn test_shutdown_request_is_idempotent() {
        let signal = ShutdownSignal::new();
        signal.request();
        signal.request();
        signal.wait();
    }

    #[test]
    fn test_parse_gui_exe_argument_keeps_the_launching_path() {
        let args = vec![
            "Synector.exe".to_string(),
            "--daemon".to_string(),
            "--spawned-by-gui".to_string(),
            "--gui-exe".to_string(),
            "C:\\Program Files\\Synector\\Synector.exe".to_string(),
        ];
        assert_eq!(
            parse_gui_exe_argument(&args),
            Some(PathBuf::from("C:\\Program Files\\Synector\\Synector.exe"))
        );
        assert_eq!(
            parse_gui_exe_argument(&[
                "Synector.exe".to_string(),
                "--daemon".to_string()
            ]),
            None
        );
        assert_eq!(
            parse_gui_exe_argument(&[
                "Synector.exe".to_string(),
                "--daemon".to_string(),
                "--gui-exe".to_string(),
                String::new()
            ]),
            None,
            "an empty value is treated as unknown, not as an empty path"
        );
        assert_eq!(
            parse_gui_exe_argument(&[
                "Synector.exe".to_string(),
                "--daemon".to_string(),
                "--gui-exe".to_string()
            ]),
            None,
            "a missing value is ignored"
        );
    }

    #[test]
    fn test_shutdown_sequence_keeps_engine_first_and_flushes_last() {
        let order = std::cell::RefCell::new(Vec::new());
        run_shutdown_sequence(
            || order.borrow_mut().push("shutdown-rules"),
            || order.borrow_mut().push("overlays"),
            || order.borrow_mut().push("persist"),
        );
        assert_eq!(
            *order.borrow(),
            vec!["shutdown-rules", "overlays", "persist"],
            "overlays must fade out before the final runtime/overlay-position flush"
        );
    }
}
