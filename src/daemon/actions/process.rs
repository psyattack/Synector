use std::collections::HashMap;
use std::process::Command;

use crate::win_args::{join_windows_arguments, split_windows_arguments};
use super::support::command::{
    apply_no_window, powershell_helper_command, ps_single_quoted, run_command_bounded_async,
    ELEVATION_TIMEOUT, UTILITY_TIMEOUT,
};
use super::get_foreground_window_pid;
use super::ActionRunner;
use crate::models::Action;

#[cfg(windows)]
use crate::models::ProcessPriorityLevel;
#[cfg(windows)]
use windows::Win32::Foundation::CloseHandle;
#[cfg(windows)]
use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
#[cfg(windows)]
use windows::Win32::System::Threading::{
    OpenProcess, SetPriorityClass, SetProcessAffinityMask,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, PROCESS_SET_QUOTA,
    REALTIME_PRIORITY_CLASS, HIGH_PRIORITY_CLASS, ABOVE_NORMAL_PRIORITY_CLASS,
    NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, IDLE_PRIORITY_CLASS,
};

pub(super) async fn execute(action: &Action, context: &HashMap<String, String>) -> Result<(), String> {
    match action {
        Action::StartProcess {
            file_path,
            arguments,
            working_directory,
            run_as_admin,
            only_if_not_running,
            run_hidden,
            ..
        } => {
            let path = ActionRunner::interpolate_tokens(file_path, context);
            let args = ActionRunner::interpolate_tokens(arguments, context);
            let argv = split_windows_arguments(&args);

            if *only_if_not_running {
                let exe_name = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or(&path)
                    .to_lowercase();

                let snapshot = crate::daemon::watcher::ProcessWatcher::snapshot_process_map();
                if snapshot.values().any(|name| name == &exe_name) {
                    return Ok(());
                }
            }

            if *run_as_admin {
                #[cfg(windows)]
                {
                    // Run as admin via powershell Start-Process -Verb RunAs. The argv is
                    // rebuilt into a quoted command line and passed as a PowerShell
                    // literal, so quoted arguments survive the two parsers intact.
                    let arg_list = join_windows_arguments(&argv);
                    let ps_args = format!(
                        "Start-Process -FilePath '{}' -ArgumentList '{}' -Verb RunAs",
                        ps_single_quoted(&path),
                        ps_single_quoted(&arg_list)
                    );
                    let out = run_command_bounded_async(
                        powershell_helper_command(&ps_args),
                        ELEVATION_TIMEOUT,
                    )
                    .await?;
                    if out.timed_out {
                        return Err(
                            "Elevated process launch timed out while waiting for the UAC prompt"
                                .to_string(),
                        );
                    }
                    if !out.succeeded() {
                        return Err(format!(
                            "Elevated process launch failed (exit code {:?})",
                            out.exit_code()
                        ));
                    }
                    return Ok(());
                }
                #[cfg(not(windows))]
                {
                    let _ = (&path, &argv);
                }
            }

            let mut cmd = Command::new(&path);
            if !argv.is_empty() {
                cmd.args(&argv);
            }
            if let Some(wd) = working_directory {
                cmd.current_dir(wd);
            }

            // Window style on Windows
            #[cfg(windows)]
            {
                if *run_hidden {
                    apply_no_window(&mut cmd);
                }
            }

            match cmd.spawn() {
                Ok(_) => Ok(()),
                Err(e) => Err(format!("Failed to spawn process '{}': {e}", path)),
            }
        }

        Action::KillProcess {
            process_name,
            kill_tree,
            force,
        } => {
            let mut name = ActionRunner::interpolate_tokens(process_name, context);
            if !name.to_lowercase().ends_with(".exe") && !name.contains('.') {
                name.push_str(".exe");
            }
            let mut cmd = Command::new("taskkill");
            if *force {
                cmd.arg("/F");
            }
            if *kill_tree {
                cmd.arg("/T");
            }
            cmd.args(["/IM", &name]);
            apply_no_window(&mut cmd);

            let out = run_command_bounded_async(cmd, UTILITY_TIMEOUT).await?;
            if out.timed_out {
                return Err(format!(
                    "taskkill timed out after {}s",
                    UTILITY_TIMEOUT.as_secs()
                ));
            }
            if !out.succeeded() {
                return Err(format!(
                    "taskkill failed with exit code {:?}",
                    out.exit_code()
                ));
            }
            Ok(())
        }

        Action::ProcessTuning {
            target_process,
            priority,
            affinity_mask,
            trim_memory,
        } => {
            let default_proc = String::new();
            let proc_name = target_process.as_ref().unwrap_or(
                context.get("ProcessName").unwrap_or(&default_proc)
            );
            let target_pid = context.get("ProcessId").and_then(|p| p.parse::<u32>().ok());
            #[cfg(windows)]
            {
                tune_process(
                    proc_name,
                    target_pid,
                    *priority,
                    *affinity_mask,
                    *trim_memory,
                )?;
            }
            Ok(())
        }

        Action::TrimWorkingSet {
            target_process,
            soft_limit_mb,
            skip_if_focused,
        } => {
            let default_proc = String::new();
            let interp = target_process.as_ref().map(|p| ActionRunner::interpolate_tokens(p, context));
            let proc_name = interp.as_deref().or_else(|| context.get("ProcessName").map(|s| s.as_str())).unwrap_or(&default_proc);
            let target_pid = context.get("ProcessId").and_then(|p| p.parse::<u32>().ok());
            trim_working_set(proc_name, target_pid, *soft_limit_mb, *skip_if_focused)?;
            Ok(())
        }
        _ => unreachable!("process handler received a non-process action"),
    }
}

#[cfg(windows)]
fn tune_process(
    target_process: &str,
    target_pid: Option<u32>,
    priority: Option<ProcessPriorityLevel>,
    affinity_mask: Option<u64>,
    trim_memory: bool,
) -> Result<(), String> {
    let mut pids: Vec<u32> = Vec::new();
    if let Some(pid) = target_pid {
        if pid > 0 {
            pids.push(pid);
        }
    }
    if pids.is_empty() && !target_process.is_empty() {
        let snapshot = crate::daemon::watcher::ProcessWatcher::snapshot_process_map();
        let target_lower = target_process.to_lowercase();
        for (pid, name) in snapshot {
            if name == target_lower {
                pids.push(pid);
            }
        }
    }

    if pids.is_empty() {
        return Err(format!("Process '{}' not found for tuning", target_process));
    }

    let mut attempted = 0u32;
    let mut failed = 0u32;
    for pid in pids {
        unsafe {
            let h_proc = OpenProcess(
                PROCESS_SET_INFORMATION | PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                pid,
            );
            let handle = match h_proc {
                Ok(handle) => handle,
                Err(e) => {
                    attempted += 1;
                    failed += 1;
                    crate::logging::warn(
                        "ACTION",
                        &format!("Process tuning: OpenProcess failed for PID {pid}: {e}"),
                    );
                    continue;
                }
            };
            if let Some(prio) = priority {
                attempted += 1;
                let class = match prio {
                    ProcessPriorityLevel::Realtime => REALTIME_PRIORITY_CLASS,
                    ProcessPriorityLevel::High => HIGH_PRIORITY_CLASS,
                    ProcessPriorityLevel::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
                    ProcessPriorityLevel::Normal => NORMAL_PRIORITY_CLASS,
                    ProcessPriorityLevel::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
                    ProcessPriorityLevel::Idle => IDLE_PRIORITY_CLASS,
                };
                if let Err(e) = SetPriorityClass(handle, class) {
                    failed += 1;
                    crate::logging::warn(
                        "ACTION",
                        &format!("Process tuning: SetPriorityClass failed for PID {pid}: {e}"),
                    );
                }
            }
            if let Some(mask) = affinity_mask {
                attempted += 1;
                if let Err(e) = SetProcessAffinityMask(handle, mask as usize) {
                    failed += 1;
                    crate::logging::warn(
                        "ACTION",
                        &format!("Process tuning: SetProcessAffinityMask failed for PID {pid}: {e}"),
                    );
                }
            }
            if trim_memory {
                attempted += 1;
                if let Err(e) = EmptyWorkingSet(handle) {
                    failed += 1;
                    crate::logging::warn(
                        "ACTION",
                        &format!("Process tuning: EmptyWorkingSet failed for PID {pid}: {e}"),
                    );
                }
            }
            let _ = CloseHandle(handle);
        }
    }
    if attempted > 0 && failed >= attempted {
        return Err(format!(
            "Process tuning failed for every target of '{}'",
            target_process
        ));
    }
    Ok(())
}

pub fn trim_working_set(
    target_process: &str,
    target_pid: Option<u32>,
    soft_limit_mb: Option<u64>,
    skip_if_focused: bool,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
        use windows::Win32::System::Memory::{SetProcessWorkingSetSizeEx, SETPROCESSWORKINGSETSIZEEX_FLAGS};
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_QUERY_LIMITED_INFORMATION};

        let mut pids: Vec<u32> = Vec::new();
        if let Some(pid) = target_pid {
            if pid > 0 {
                pids.push(pid);
            }
        }
        if pids.is_empty() && !target_process.is_empty() {
            let snapshot = crate::daemon::watcher::ProcessWatcher::snapshot_process_map();
            let target_lower = target_process.to_lowercase();
            let target_has_exe = target_lower.ends_with(".exe");
            for (pid, name) in snapshot {
                if name == target_lower || (!target_has_exe && name == format!("{target_lower}.exe")) {
                    pids.push(pid);
                }
            }
        }
        if pids.is_empty() && target_process.is_empty() {
            if let Some(fg_pid) = get_foreground_window_pid() {
                if fg_pid != std::process::id() {
                    pids.push(fg_pid);
                }
            }
        }
        if pids.is_empty() {
            return Err(format!("Process '{}' not found for trimming working set", target_process));
        }

        if skip_if_focused {
            if let Some(fg_pid) = get_foreground_window_pid() {
                if pids.contains(&fg_pid) {
                    return Ok(());
                }
            }
        }

        let mut attempted = 0u32;
        let mut failed = 0u32;
        for pid in pids {
            if skip_if_focused {
                if let Some(fg_pid) = get_foreground_window_pid() {
                    if pid == fg_pid {
                        continue;
                    }
                }
            }

            unsafe {
                match OpenProcess(PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                    Ok(handle) => {
                        if let Some(mb) = soft_limit_mb {
                            attempted += 1;
                            let max_bytes = (mb as usize).saturating_mul(1024 * 1024).max(2 * 1024 * 1024);
                            let min_bytes = 1024 * 1024;
                            if let Err(e) = SetProcessWorkingSetSizeEx(
                                handle,
                                min_bytes,
                                max_bytes,
                                SETPROCESSWORKINGSETSIZEEX_FLAGS(0),
                            ) {
                                failed += 1;
                                crate::logging::warn(
                                    "ACTION",
                                    &format!("Trim working set: SetProcessWorkingSetSizeEx failed for PID {pid}: {e}"),
                                );
                            }
                        } else {
                            attempted += 1;
                            if let Err(e) = EmptyWorkingSet(handle) {
                                failed += 1;
                                crate::logging::warn(
                                    "ACTION",
                                    &format!("Trim working set: EmptyWorkingSet failed for PID {pid}: {e}"),
                                );
                            }
                        }
                        let _ = CloseHandle(handle);
                    }
                    Err(e) => {
                        attempted += 1;
                        failed += 1;
                        crate::logging::warn(
                            "ACTION",
                            &format!("Trim working set: OpenProcess failed for PID {pid}: {e}"),
                        );
                    }
                }
            }
        }
        if attempted > 0 && failed >= attempted {
            return Err(format!(
                "Trimming the working set failed for every target of '{}'",
                target_process
            ));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (target_process, target_pid, soft_limit_mb, skip_if_focused);
    }
    Ok(())
}
