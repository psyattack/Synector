use std::collections::HashMap;
use std::process::Command;

use crate::win_args::{join_windows_arguments, split_windows_arguments};
use super::support::command::{
    apply_no_window, powershell_helper_command, ps_single_quoted, run_command_bounded_async,
    ELEVATION_TIMEOUT, UTILITY_TIMEOUT,
};
use super::get_foreground_window_pid;
use super::ActionRunner;
use crate::models::{Action, ResourceLifetime};

#[cfg(windows)]
use crate::models::ProcessPriorityLevel;
#[cfg(windows)]
use windows::Win32::Foundation::CloseHandle;
#[cfg(windows)]
use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
#[cfg(windows)]
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, SetPriorityClass, SetProcessAffinityMask,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, PROCESS_SET_QUOTA,
    REALTIME_PRIORITY_CLASS, HIGH_PRIORITY_CLASS, ABOVE_NORMAL_PRIORITY_CLASS,
    NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, IDLE_PRIORITY_CLASS,
};

/// One process that a `start_process` step actually spawned, with its captured PID.
pub(super) struct SpawnedProcess {
    pub(super) pid: u32,
    pub(super) image_path: String,
}

/// Spawns a process for a `start_process` action and reports the child it created.
///
/// A skipped `only_if_not_running` spawn returns `None`: the pipeline must never take ownership of
/// a process it did not start. An elevated launch goes through PowerShell, so its PID comes from
/// `Start-Process -PassThru`; if the PID cannot be read, an owning lifetime fails the step instead
/// of tracking nothing.
pub(super) async fn execute_start(
    action: &Action,
    context: &HashMap<String, String>,
) -> Result<Option<SpawnedProcess>, String> {
    let Action::StartProcess {
        file_path,
        arguments,
        working_directory,
        run_as_admin,
        only_if_not_running,
        run_hidden,
        lifetime,
        ..
    } = action
    else {
        return Err("start_process handler received a non-start action".to_string());
    };
    let path = ActionRunner::interpolate_tokens(file_path, context);
    let args = ActionRunner::interpolate_tokens(arguments, context);
    let argv = split_windows_arguments(&args);
    let need_pid = *lifetime != ResourceLifetime::Detach;

    if *only_if_not_running {
        let exe_name = std::path::Path::new(&path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(&path)
            .to_lowercase();

        let snapshot = crate::daemon::watcher::ProcessWatcher::snapshot_process_map();
        if snapshot.values().any(|name| name == &exe_name) {
            return Ok(None);
        }
    }

    if *run_as_admin {
        #[cfg(windows)]
        {
            // Run as admin via powershell Start-Process -Verb RunAs. The argv is
            // rebuilt into a quoted command line and passed as a PowerShell
            // literal, so quoted arguments survive the two parsers intact. The
            // -PassThru process id is the only handle the pipeline can own.
            let arg_list = join_windows_arguments(&argv);
            let ps_args = format!(
                "$p = Start-Process -FilePath '{}' -ArgumentList '{}' -Verb RunAs -PassThru; if ($p) {{ $p.Id }}",
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
            let pid = out
                .stdout
                .lines()
                .rev()
                .find_map(|line| line.trim().parse::<u32>().ok());
            return match pid {
                Some(pid) => Ok(Some(SpawnedProcess {
                    pid,
                    image_path: path,
                })),
                None if need_pid => Err(
                    "The elevated process started, but its process id could not be determined"
                        .to_string(),
                ),
                None => Ok(None),
            };
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
        Ok(child) => Ok(Some(SpawnedProcess {
            pid: child.id(),
            image_path: path,
        })),
        Err(e) => Err(format!("Failed to spawn process '{}': {e}", path)),
    }
}

/// Stops one owned process and every descendant by its captured PID.
///
/// The PID is the identity tracked at spawn time; unlike the old name-based teardown it cannot
/// hit an unrelated process with the same executable name. `taskkill /T` covers the tree.
/// A `taskkill` command resolved under `%SystemRoot%\System32` when possible, so a planted
/// `taskkill.exe` in the executable's directory or the inherited working directory cannot run with
/// the daemon's rights.
fn taskkill_command() -> Command {
    let path = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .map(|root| std::path::PathBuf::from(root).join("System32").join("taskkill.exe"))
        .filter(|path| path.is_file());
    match path {
        Some(path) => Command::new(path),
        None => Command::new("taskkill"),
    }
}

pub(crate) async fn kill_process_tree_by_pid(pid: u32) -> Result<(), String> {
    let mut cmd = taskkill_command();
    cmd.args(["/PID", &pid.to_string(), "/T", "/F"]);
    apply_no_window(&mut cmd);

    let out = run_command_bounded_async(cmd, UTILITY_TIMEOUT).await?;
    if out.timed_out {
        return Err(format!(
            "taskkill timed out after {}s while stopping process {pid}",
            UTILITY_TIMEOUT.as_secs()
        ));
    }
    if !out.succeeded() {
        return Err(format!(
            "taskkill failed with exit code {:?} while stopping process {pid}",
            out.exit_code()
        ));
    }
    Ok(())
}

/// Blocking form of [`kill_process_tree_by_pid`] for callers outside a Tokio runtime.
pub(crate) fn kill_process_tree_by_pid_blocking(pid: u32) -> Result<(), String> {
    let mut cmd = taskkill_command();
    cmd.args(["/PID", &pid.to_string(), "/T", "/F"]);
    apply_no_window(&mut cmd);
    let out = crate::command::run_command_bounded(cmd, UTILITY_TIMEOUT)?;
    out.ensure_success("taskkill", UTILITY_TIMEOUT)
}

/// Whether `pid` still names a running process.
///
/// An access-denied probe counts as alive so teardown is never skipped for an elevated process;
/// only `ERROR_INVALID_PARAMETER` (the pid no longer exists) and a known exit code report death.
pub(crate) fn is_process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        const ERROR_INVALID_PARAMETER: u32 = 87;
        const STILL_ACTIVE: u32 = 259;

        if pid == 0 {
            return false;
        }
        let handle = match unsafe {
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
        } {
            Ok(handle) => handle,
            Err(err) => {
                return (err.code().0 as u32 & 0xFFFF) != ERROR_INVALID_PARAMETER;
            }
        };
        let mut code = 0u32;
        let alive = unsafe { GetExitCodeProcess(handle, &mut code) }
            .is_ok_and(|()| code == STILL_ACTIVE);
        let _ = unsafe { CloseHandle(handle) };
        alive
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

pub(super) async fn execute(action: &Action, context: &HashMap<String, String>) -> Result<(), String> {
    match action {
        Action::KillProcess {
            process_name,
            kill_tree,
            force,
        } => {
            let mut name = ActionRunner::interpolate_tokens(process_name, context);
            if !name.to_lowercase().ends_with(".exe") && !name.contains('.') {
                name.push_str(".exe");
            }
            let mut cmd = taskkill_command();
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
    Ok(())
}
