use std::path::Path;

use super::portable::swap_pending_executable;
use super::util::{escape_ps_literal, system32_tool};

/// Stops Synector processes before an update replaces files. Returns true when the GUI was running
/// and should be restored, false for a daemon-only restart. The daemon is the only update
/// initiator: the GUI triggers updates over IPC, so the caller always runs in the daemon.
pub(super) fn shutdown_for_update() -> bool {
    let gui_running = crate::single_instance::is_gui_running();
    set_gui_closed_during_update(!gui_running);
    if gui_running {
        crate::single_instance::close_existing_gui_window();
    }
    // Cleanly and synchronously dismiss all overlays before replacing binaries.
    crate::daemon::overlay_manager::hide_all_sync();
    std::thread::sleep(std::time::Duration::from_millis(200));
    gui_running
}

static GUI_CLOSED_DURING_UPDATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_gui_closed_during_update(closed: bool) {
    GUI_CLOSED_DURING_UPDATE.store(closed, std::sync::atomic::Ordering::SeqCst);
}

pub fn is_gui_closed_during_update() -> bool {
    GUI_CLOSED_DURING_UPDATE.load(std::sync::atomic::Ordering::SeqCst)
}

/// Performs executable swap and restarts both daemon and GUI.
///
/// The in-place path renames the running executable aside and moves the staged one into its place,
/// then starts the new process from a detached PowerShell script and exits.
pub fn apply_update_and_restart(exe_path: &Path, pending_path: &Path) -> Result<(), String> {
    crate::logging::info("UPDATER", "Applying update and restarting Synector");
    let mut old_path = exe_path.with_extension("old");
    if old_path.exists() && std::fs::remove_file(&old_path).is_err() {
        old_path = exe_path.with_extension(format!("old.{}", std::process::id()));
    }

    let gui_pid = std::process::id();
    let old_esc = escape_ps_literal(&old_path.to_string_lossy());
    let exe_esc = escape_ps_literal(&exe_path.to_string_lossy());
    let pending_esc = escape_ps_literal(&pending_path.to_string_lossy());

    let restart_args = if is_gui_closed_during_update() {
        " -ArgumentList '--daemon'"
    } else {
        ""
    };

    let focus_post_start = if is_gui_closed_during_update() {
        "".to_string()
    } else {
        "; Start-Sleep -Milliseconds 700; (New-Object -ComObject WScript.Shell).AppActivate('Synector')".to_string()
    };

    let in_place_renamed = swap_pending_executable(exe_path, pending_path, &old_path)?;

    let ps_command = if in_place_renamed {
        // Start the new executable before deleting the old one so a startup failure still leaves
        // the previous binary on disk.
        format!(
            "Wait-Process -Id {gui_pid} -Timeout 10 -ErrorAction SilentlyContinue; \
             Start-Sleep -Milliseconds 600; \
             $started = Start-Process -FilePath '{exe_esc}'{restart_args} -PassThru -ErrorAction SilentlyContinue; \
             if ($started) {{ Remove-Item -LiteralPath '{old_esc}' -Force -ErrorAction SilentlyContinue{focus_post_start} }} \
             else {{ Move-Item -LiteralPath '{old_esc}' -Destination '{exe_esc}' -Force -ErrorAction SilentlyContinue; Start-Process -FilePath '{exe_esc}'{restart_args} }}",
        )
    } else {
        // The executable could not be renamed while running: retry from the detached script once
        // this process has exited. The previous binary is kept until the new one is in place and
        // started, and a failed move restores it instead of deleting the only copy.
        format!(
            "Wait-Process -Id {gui_pid} -Timeout 10 -ErrorAction SilentlyContinue; \
             Get-Process -Name 'Synector' -ErrorAction SilentlyContinue | Where-Object {{ $_.Id -ne $PID }} | Stop-Process -Force -ErrorAction SilentlyContinue; \
             Start-Sleep -Milliseconds 400; \
             $sw = [System.Diagnostics.Stopwatch]::StartNew(); \
             while ($sw.ElapsedMilliseconds -lt 6000) {{ \
                 if (Test-Path -LiteralPath '{pending_esc}') {{ \
                     if (Test-Path -LiteralPath '{exe_esc}') {{ \
                         Remove-Item -LiteralPath '{old_esc}' -Force -ErrorAction SilentlyContinue; \
                         Move-Item -LiteralPath '{exe_esc}' -Destination '{old_esc}' -Force -ErrorAction SilentlyContinue; \
                     }} \
                     Move-Item -LiteralPath '{pending_esc}' -Destination '{exe_esc}' -Force -ErrorAction SilentlyContinue; \
                 }} \
                 if (Test-Path -LiteralPath '{exe_esc}') {{ break; }} \
                 if ((Test-Path -LiteralPath '{old_esc}') -and -not (Test-Path -LiteralPath '{exe_esc}')) {{ \
                     Move-Item -LiteralPath '{old_esc}' -Destination '{exe_esc}' -Force -ErrorAction SilentlyContinue; \
                 }} \
                 Start-Sleep -Milliseconds 250; \
             }} \
             $started = Start-Process -FilePath '{exe_esc}'{restart_args} -PassThru -ErrorAction SilentlyContinue; \
             if ($started) {{ Remove-Item -LiteralPath '{old_esc}' -Force -ErrorAction SilentlyContinue{focus_post_start} }}",
        )
    };

    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }

    #[cfg(windows)]
    {
        let powershell = system32_tool("powershell.exe")?;
        let mut cmd = std::process::Command::new(powershell);
        cmd.args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &ps_command]);
        crate::win_args::apply_no_window(&mut cmd);
        cmd.spawn().map_err(|e| format!("Failed to spawn restart script: {e}"))?;
    }

    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new(exe_path).spawn();
    }

    std::process::exit(0);
}
