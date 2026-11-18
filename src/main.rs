#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod autorun;
mod cli;
pub mod config;
mod daemon;
mod gui;
pub mod i18n;
mod ipc;
pub mod logging;
pub mod models;
pub mod sharing;
pub mod single_instance;
pub mod shortcut;
pub mod updater;

use std::env;
use std::process::Command;
use std::thread;
use std::time::Duration;
use ipc::IpcClient;

fn main() {
    logging::init();
    logging::install_panic_hook();

    let args: Vec<String> = env::args().collect();

    // 1. Fast-path: standalone overlay window processes should NEVER touch update files or single instance mutexes
    if let Some(pos) = args.iter().position(|a| a == "--overlay-window" || a == "-o") {
        let overlay_id = args.get(pos + 1).cloned().unwrap_or_default();
        let calibration = args.iter().any(|a| a == "--calibration");
        let duration_ms = args.iter().position(|a| a == "--duration").and_then(|p| args.get(p + 1)).and_then(|s| s.parse::<u32>().ok());
        let override_text = args.iter().position(|a| a == "--override-text").and_then(|p| args.get(p + 1)).cloned();
        let overlay_data = args.iter().position(|a| a == "--overlay-data").and_then(|p| args.get(p + 1)).cloned();
        if let Err(e) = gui::overlay_window::run_overlay_window(overlay_id, calibration, override_text, duration_ms, overlay_data) {
            logging::error("OVERLAY", &format!("Failed to launch overlay window: {e}"));
            eprintln!("Failed to launch overlay window: {e}");
        }
        return;
    }

    // 2. Rule authoring commands are console tools: run them before elevation and update cleanup
    if cli::run(&args) {
        return;
    }

    // Clean up temporary update files ONLY if daemon is not running (to avoid deleting active downloads)
    if let Ok(exe_path) = env::current_exe() {
        let old = exe_path.with_extension("old");
        if old.exists() {
            let _ = std::fs::remove_file(old);
        }
    }
    if !single_instance::is_daemon_running() {
        updater::clean_temp_update_files();
    }

    let is_daemon_flag = args.iter().any(|a| {
        a == "--daemon" || a == "-d" || a == "--service" || a == "-s"
    });

    if args.iter().any(|a| a == "--status") {
        match IpcClient::call(&ipc::IpcRequest::GetState) {
            Ok(ipc::IpcResponse::State(state)) => {
                println!("DAEMON STATUS: Online");
                println!("  Rules: {} (active: {})", state.total_rules_count, state.active_rules_count);
                println!("  Daemon Memory: {:.2} MB", state.daemon_memory_bytes as f64 / 1_048_576.0);
            }
            Ok(other) => println!("Daemon response: {:?}", other),
            Err(e) => println!("DAEMON STATUS: Offline ({e})"),
        }
        return;
    }

    // Direct rule execution via CLI or Windows shortcut: --run-rule <rule_id>
    if let Some(pos) = args.iter().position(|a| a == "--run-rule" || a == "-r") {
        let rule_id = args.get(pos + 1).cloned().unwrap_or_default();
        if !rule_id.is_empty() {
            run_rule_cli(&rule_id);
            return;
        }
    }

    // Auto-elevate to Administrator rights by default for system-wide ETW & UIPI input control
    let no_elevate = args.iter().any(|a| a == "--no-elevate");
    #[cfg(windows)]
    if !no_elevate && try_self_elevate() {
        logging::info("APP", "Not elevated: relaunching Synector as administrator");
        return;
    }

    if is_daemon_flag {
        daemon::run_daemon();
    } else {
        // Both default execution and --gui ensure background daemon is running before opening GUI
        ensure_daemon_running();
        if let Err(e) = gui::run_gui() {
            logging::error("GUI", &format!("Failed to launch GUI: {e}"));
            eprintln!("Failed to launch GUI: {e}");
        }
    }
}

pub fn ensure_daemon_running() {
    if IpcClient::is_daemon_running() {
        logging::debug("DAEMON", "Background daemon is already running");
        return;
    }

    let mask_daemon_process = config::Settings::read().mask_daemon_process;
    let exe_path = if mask_daemon_process {
        daemon::pe_mask::prepare_masked_daemon().unwrap_or_else(|err| {
            logging::warn(
                "DAEMON",
                &format!("Could not prepare masked daemon image: {err}"),
            );
            env::current_exe().unwrap_or_default()
        })
    } else {
        env::current_exe().unwrap_or_default()
    };

    if exe_path.exists() {
        let mut cmd = Command::new(&exe_path);
        cmd.arg("--daemon");
        cmd.arg("--spawned-by-gui");
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        match cmd.spawn() {
            Ok(_) => logging::info(
                "DAEMON",
                &format!("Spawned background daemon: {}", exe_path.display()),
            ),
            Err(err) => logging::error(
                "DAEMON",
                &format!("Failed to spawn background daemon: {err}"),
            ),
        }

        // Wait up to 2 seconds for pipe to be available
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(100));
            if IpcClient::is_daemon_running() {
                break;
            }
        }
    } else {
        logging::error(
            "DAEMON",
            &format!(
                "Daemon executable path does not exist: {}",
                exe_path.display()
            ),
        );
    }
}

#[cfg(windows)]
fn try_self_elevate() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::SW_NORMAL;
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOASYNC};
    use windows::core::{HSTRING, PCWSTR};

    if daemon::watcher::is_elevated() {
        return false;
    }

    let exe_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };

    let args: Vec<String> = std::env::args().skip(1).collect();
    let args_str = args.iter().map(|a| {
        if a.contains(' ') {
            format!("\"{}\"", a)
        } else {
            a.clone()
        }
    }).collect::<Vec<_>>().join(" ");

    let verb = HSTRING::from("runas");
    let file = HSTRING::from(exe_path.to_string_lossy().to_string());
    let params = HSTRING::from(args_str);

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_NORMAL.0,
        ..Default::default()
    };

    unsafe {
        ShellExecuteExW(&mut info).is_ok()
    }
}

pub fn run_rule_cli(rule_id: &str) {
    logging::info("CLI", &format!("Direct rule run requested: {rule_id}"));

    // 1. Try sending via IPC if background daemon is already running
    if single_instance::is_daemon_running() || IpcClient::is_daemon_running() {
        if let Ok(ipc::IpcResponse::Ok) = IpcClient::call(&ipc::IpcRequest::TestRunRule {
            id: rule_id.to_string(),
            rule: None,
        }) {
            println!("Rule '{rule_id}' sent to running daemon for execution.");
            logging::info("CLI", &format!("Rule '{rule_id}' sent to the running daemon"));
            return;
        }
    }

    // 2. Autonomous execution when daemon is offline
    let cfg = config::AppConfig::read();
    if let Some(rule) = cfg.rules.iter().find(|r| r.id == rule_id) {
        println!("Running rule '{}' ({}) autonomously...", rule.name, rule.id);
        logging::info(
            "CLI",
            &format!("Running rule '{}' ({}) without a daemon", rule.name, rule.id),
        );
        match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(rt) => rt.block_on(async {
                let mut ctx = std::collections::HashMap::new();
                for (step_idx, action) in rule.actions.iter().enumerate() {
                    match daemon::actions::ActionRunner::execute_with_output(action, &ctx).await {
                        Ok(daemon::actions::ActionOutcome::Continue(out_opt)) => {
                            if let Some(out) = out_opt {
                                ctx.insert("PrevOutput".to_string(), out.clone());
                                ctx.insert(format!("Step{}.Output", step_idx + 1), out);
                            }
                        }
                        Ok(daemon::actions::ActionOutcome::AbortPipeline { reason }) => {
                            println!("Pipeline gracefully halted at step {}: {}", step_idx + 1, reason);
                            logging::info(
                                "CLI",
                                &format!(
                                    "Rule '{}' pipeline halted at step {}: {}",
                                    rule.id,
                                    step_idx + 1,
                                    reason
                                ),
                            );
                            break;
                        }
                        Err(err) => {
                            eprintln!("Action at step {} failed: {}", step_idx + 1, err);
                            logging::error(
                                "CLI",
                                &format!(
                                    "Rule '{}' action at step {} failed: {err}",
                                    rule.id,
                                    step_idx + 1
                                ),
                            );
                            if rule.stop_on_error {
                                logging::warn(
                                    "CLI",
                                    &format!(
                                        "Rule '{}' pipeline stopped at step {} due to error",
                                        rule.id,
                                        step_idx + 1
                                    ),
                                );
                                break;
                            }
                        }
                    }
                }
            }),
            Err(err) => logging::error(
                "CLI",
                &format!("Failed to create Tokio runtime for the direct rule run: {err}"),
            ),
        }
        logging::info("CLI", &format!("Direct rule run finished: {rule_id}"));
    } else {
        eprintln!("Rule with ID '{rule_id}' was not found in configuration.");
        logging::warn(
            "CLI",
            &format!("Rule '{rule_id}' was not found in configuration"),
        );
    }
}
