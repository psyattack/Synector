use super::launch::{launching_gui_exe, record_launching_gui_exe};
use super::loops;
use super::shutdown::{request_shutdown, run_shutdown_sequence, wait_for_shutdown_request};
use crate::config::AppConfig;
use crate::daemon::engine::DaemonEngine;
use crate::daemon::hotkeys;
use crate::daemon::ipc_server::IpcServer;
use crate::daemon::overlay_manager;
use crate::daemon::pe_mask;
use crate::daemon::rule_call;
use crate::daemon::tray::TrayManager;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) fn run() {
    record_launch();

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

    log_startup();

    let config = prepare_state();

    let engine = Arc::new(Mutex::new(DaemonEngine::new(config.clone())));

    let Some(rt) = build_runtime() else {
        return;
    };

    install_rule_control_dispatcher(&rt, Arc::clone(&engine));

    // Initialize Global Hotkey Listener
    let hotkey_rx = install_hotkeys(&rt, Arc::clone(&engine), &config);

    // Spawn Global Hotkey Event Loop (supports Normal toggle, Hold mode, and Infinite macro cancellation)
    let engine_hotkey = Arc::clone(&engine);
    let hold_tasks: Arc<
        tokio::sync::Mutex<std::collections::HashMap<String, tokio::task::AbortHandle>>,
    > = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

    let hold_tasks_loop = Arc::clone(&hold_tasks);
    rt.spawn(loops::spawn_hotkey_loop(engine_hotkey, hotkey_rx, hold_tasks_loop));

    log_initialized(&rt, Arc::clone(&engine));

    spawn_ipc_server(&rt, Arc::clone(&engine));

    spawn_loops(&rt, Arc::clone(&engine), &config);

    log_service_started();

    run_tray_or_headless(&rt, Arc::clone(&engine), &config);

    shutdown(&rt, engine);
}

fn record_launch() {
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
}

fn log_startup() {
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
}

fn prepare_state() -> AppConfig {
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

    config
}

fn build_runtime() -> Option<tokio::runtime::Runtime> {
    // Create multi-threaded Tokio runtime for background tasks
    match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => Some(rt),
        Err(err) => {
            crate::logging::error(
                "DAEMON",
                &format!("Failed to create the Tokio runtime: {err}"),
            );
            None
        }
    }
}

fn install_rule_control_dispatcher(rt: &tokio::runtime::Runtime, engine: Arc<Mutex<DaemonEngine>>) {
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
}

fn install_hotkeys(
    rt: &tokio::runtime::Runtime,
    engine: Arc<Mutex<DaemonEngine>>,
    config: &AppConfig,
) -> tokio::sync::mpsc::UnboundedReceiver<hotkeys::HotkeyEvent> {
    let (hotkey_tx, hotkey_rx) = tokio::sync::mpsc::unbounded_channel::<hotkeys::HotkeyEvent>();
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

    hotkey_rx
}

fn log_initialized(rt: &tokio::runtime::Runtime, engine: Arc<Mutex<DaemonEngine>>) {
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
}

fn spawn_ipc_server(rt: &tokio::runtime::Runtime, engine: Arc<Mutex<DaemonEngine>>) {
    // Spawn IPC Server
    let ipc_server = IpcServer::new(Arc::clone(&engine));
    rt.spawn(async move {
        ipc_server.run().await;
    });
}

fn spawn_loops(
    rt: &tokio::runtime::Runtime,
    engine: Arc<Mutex<DaemonEngine>>,
    config: &AppConfig,
) {
    // Flush coalesced runtime counters (execution counts, last-run timestamps) and
    // overlay drag positions so hot paths write at a bounded rate instead of per event.
    let engine_runtime = Arc::clone(&engine);
    rt.spawn(loops::spawn_runtime_flush_loop(engine_runtime));

    // Spawn Process Watcher Loop (Hybrid: ETW 50ms queue drain under admin; Adaptive 150-300ms snapshot polling; Dormant when no rules)
    let engine_process = Arc::clone(&engine);
    rt.spawn(loops::spawn_process_loop(engine_process));

    // Spawn Window Focus Watcher Loop (Independent regular polling cycle 150ms)
    let engine_window = Arc::clone(&engine);
    rt.spawn(loops::spawn_window_loop(engine_window));

    // Spawn System Events Watcher Loop. Polled sources run on a timer; the notification
    // window wakes the loop immediately for suspend/resume, display and device events.
    let engine_sys = Arc::clone(&engine);
    rt.spawn(loops::spawn_system_loop(engine_sys));

    // Spawn Schedule & Resource Watcher Loop (Dormant when no schedule or resource rules exist)
    let engine_sched = Arc::clone(&engine);
    rt.spawn(loops::spawn_schedule_loop(engine_sched));

    // Spawn Webhook Server Loop
    let engine_webhook = Arc::clone(&engine);
    rt.spawn(loops::spawn_webhook_loop(engine_webhook));

    // Spawn File Watcher Loop (Dormant when no file watcher rules exist)
    let engine_file = Arc::clone(&engine);
    rt.spawn(loops::spawn_file_loop(engine_file));

    // Background update check on daemon startup
    if config.check_updates_on_startup {
        let engine_update = Arc::clone(&engine);
        let is_daemon_cli = std::env::args()
            .any(|a| crate::arg_spellings::DAEMON_MODE.contains(&a.as_str()))
            && !std::env::args().any(|a| a == crate::arg_spellings::SPAWNED_BY_GUI);
        let auto_install = config.auto_install_updates;

        rt.spawn(loops::spawn_update_check(
            engine_update,
            is_daemon_cli,
            auto_install,
        ));
    }
}

fn log_service_started() {
    crate::logging::info(
        "DAEMON",
        "Service started, listening on the Synector named pipe",
    );
    println!("[DAEMON] Service started. 0.0% CPU, listening on Named Pipe.");
}

fn run_tray_or_headless(
    rt: &tokio::runtime::Runtime,
    engine: Arc<Mutex<DaemonEngine>>,
    config: &AppConfig,
) {
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
}

fn shutdown(rt: &tokio::runtime::Runtime, engine: Arc<Mutex<DaemonEngine>>) {
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
