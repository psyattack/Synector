use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const APP_NAME: &str = "Synector";

/// Task Scheduler name for the masked autostart entry. It is namespaced so a foreign task with a
/// generic service-like name can never be mistaken for, or deleted as, Synector's own task.
pub const MASKED_TASK_NAME: &str = "Synector_DaemonMasked";

/// Masked task name used before the namespaced name was introduced. It is migrated away, but only
/// after the task definition proves that it belongs to Synector.
const LEGACY_MASKED_TASK_NAME: &str = "WindowsServiceHost";

/// How long a verified autostart status is reused. `is_autorun_enabled` is called from every state
/// snapshot the GUI polls, and verification has to read task definitions with `schtasks`.
const AUTORUN_STATUS_TTL: Duration = Duration::from_secs(5);

static AUTORUN_STATUS_CACHE: Mutex<Option<(String, Instant, bool)>> = Mutex::new(None);

struct SchtasksResult {
    success: bool,
    stdout: String,
    stderr: String,
}

fn run_schtasks(args: &[&str]) -> Result<SchtasksResult, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut cmd = std::process::Command::new("schtasks");
    cmd.args(args);
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run schtasks: {e}"))?;
    Ok(SchtasksResult {
        success: output.status.success(),
        stdout: decode_schtasks_output(&output.stdout),
        stderr: decode_schtasks_output(&output.stderr).trim().to_string(),
    })
}

fn schtasks_error(context: &str, result: &SchtasksResult) -> String {
    let detail = result.stderr.trim();
    if detail.is_empty() {
        format!("{context}: schtasks exited with an error and reported no details")
    } else {
        format!("{context}: {detail}")
    }
}

/// Decodes `schtasks` output, which is UTF-16LE on current Windows versions but is written as
/// UTF-8 when redirected in some builds. Task XML uses the same encoding rules.
fn decode_schtasks_output(bytes: &[u8]) -> String {
    fn utf16_le(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes
            .chunks(2)
            .filter(|chunk| chunk.len() == 2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    }
    fn utf16_be(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes
            .chunks(2)
            .filter(|chunk| chunk.len() == 2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    }

    if bytes.starts_with(&[0xFF, 0xFE]) {
        return utf16_le(&bytes[2..]);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return utf16_be(&bytes[2..]);
    }
    if bytes.len() >= 2 && bytes[0] != 0 && bytes[1] == 0 {
        return utf16_le(bytes);
    }
    String::from_utf8_lossy(bytes)
        .trim_start_matches('\u{feff}')
        .to_string()
}

fn unescape_xml(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Extracts the executable path from a Task Scheduler XML definition.
fn parse_task_command(xml: &str) -> Option<String> {
    let lower = xml.to_ascii_lowercase();
    let start = lower.find("<command>")? + "<command>".len();
    let tail = &xml[start..];
    let end = tail.to_ascii_lowercase().find("</command>")?;
    let command = unescape_xml(tail[..end].trim());
    let command = command.trim().trim_matches('"').trim();
    (!command.is_empty()).then(|| command.to_string())
}

fn scheduled_task_command(task_name: &str) -> Option<PathBuf> {
    let result = run_schtasks(&["/query", "/tn", task_name, "/xml"]).ok()?;
    if !result.success {
        return None;
    }
    parse_task_command(&result.stdout).map(PathBuf::from)
}

fn task_exists(task_name: &str) -> bool {
    run_schtasks(&["/query", "/tn", task_name])
        .map(|result| result.success)
        .unwrap_or(false)
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    fn normalize(path: &Path) -> String {
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    }
    !a.as_os_str().is_empty() && !b.as_os_str().is_empty() && normalize(a) == normalize(b)
}

fn masked_daemon_path() -> PathBuf {
    crate::daemon::pe_mask::get_masked_daemon_exe()
}

/// Returns whether a task command points at a Synector executable: the running copy, the masked
/// daemon image, or another copy of the same executable file name (portable/renamed installs).
fn command_belongs_to_synector(command: &Path, current_exe: &Path, masked_exe: &Path) -> bool {
    if command.as_os_str().is_empty() {
        return false;
    }
    if paths_equal(command, current_exe) || paths_equal(command, masked_exe) {
        return true;
    }
    let Some(current_name) = current_exe.file_name() else {
        return false;
    };
    command
        .file_name()
        .map(|name| name.eq_ignore_ascii_case(current_name))
        .unwrap_or(false)
}

fn task_belongs_to_synector(task_name: &str) -> bool {
    let Some(command) = scheduled_task_command(task_name) else {
        return false;
    };
    let Ok(current_exe) = std::env::current_exe() else {
        return false;
    };
    command_belongs_to_synector(&command, &current_exe, &masked_daemon_path())
}

fn is_autorun_enabled_uncached(app_name: &str) -> bool {
    [app_name, MASKED_TASK_NAME, LEGACY_MASKED_TASK_NAME]
        .iter()
        .any(|task_name| task_belongs_to_synector(task_name))
}

pub fn is_autorun_enabled(app_name: &str) -> bool {
    {
        let cache = AUTORUN_STATUS_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((cached_app, checked_at, enabled)) = cache.as_ref() {
            if cached_app == app_name && checked_at.elapsed() < AUTORUN_STATUS_TTL {
                return *enabled;
            }
        }
    }

    let enabled = is_autorun_enabled_uncached(app_name);
    let mut cache = AUTORUN_STATUS_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *cache = Some((app_name.to_string(), Instant::now(), enabled));
    enabled
}

fn invalidate_autorun_status_cache() {
    *AUTORUN_STATUS_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// Repoints the autostart task at the current executable after Synector moved (for example when
/// a portable copy was replaced by the installer). The task's presence stays the source of truth
/// for whether autostart is enabled; it is only rewritten when its target is stale.
pub fn ensure_autorun_target_current(mask_daemon_process: bool) {
    if !is_autorun_enabled(APP_NAME) {
        return;
    }
    let expected = if mask_daemon_process {
        crate::daemon::pe_mask::get_masked_daemon_exe()
    } else {
        match std::env::current_exe() {
            Ok(path) => path,
            Err(_) => return,
        }
    };
    let task_name = if mask_daemon_process {
        MASKED_TASK_NAME
    } else {
        APP_NAME
    };
    if let Some(command) = scheduled_task_command(task_name) {
        if paths_equal(&command, &expected) {
            return;
        }
    }
    crate::logging::info(
        "AUTORUN",
        &format!("Repointing the autostart task to {}", expected.display()),
    );
    if let Err(err) = set_autorun_with_mask(APP_NAME, true, mask_daemon_process) {
        crate::logging::warn(
            "AUTORUN",
            &format!("Failed to update the autostart task: {err}"),
        );
    }
}

/// Refuses to let `/create /f` silently replace a same-named task that is not Synector's own.
fn ensure_task_replaceable(task_name: &str) -> Result<(), String> {
    if task_exists(task_name) && !task_belongs_to_synector(task_name) {
        return Err(format!(
            "A scheduled task named '{task_name}' already exists and does not belong to Synector; refusing to replace it"
        ));
    }
    Ok(())
}

/// Deletes one Synector-owned task. Foreign tasks are left untouched (and logged), and every
/// delete result is checked instead of being reported to the GUI as success.
fn remove_synector_task(task_name: &str) -> Result<(), String> {
    if !task_exists(task_name) {
        return Ok(());
    }
    if !task_belongs_to_synector(task_name) {
        crate::logging::warn(
            "AUTORUN",
            &format!("Leaving scheduled task '{task_name}' alone: it does not belong to Synector"),
        );
        return Ok(());
    }
    let result = run_schtasks(&["/delete", "/tn", task_name, "/f"])?;
    if !result.success {
        return Err(schtasks_error(
            &format!("Failed to delete the Task Scheduler task '{task_name}'"),
            &result,
        ));
    }
    if task_exists(task_name) {
        return Err(format!(
            "Task Scheduler task '{task_name}' still exists after deletion"
        ));
    }
    Ok(())
}

fn enable_autostart(active_name: &str, mask_daemon_process: bool) -> Result<(), String> {
    let exe_path = if mask_daemon_process {
        crate::daemon::pe_mask::prepare_masked_daemon()
            .unwrap_or_else(|_| std::env::current_exe().unwrap_or_default())
    } else {
        std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?
    };
    if exe_path.as_os_str().is_empty() {
        return Err("Could not resolve the Synector executable path".to_string());
    }

    ensure_task_replaceable(active_name)?;
    // Remove the Synector-owned copy under any other task name (switching mask mode, or the
    // legacy generic name) after verifying ownership.
    for task_name in [APP_NAME, MASKED_TASK_NAME, LEGACY_MASKED_TASK_NAME] {
        if task_name != active_name {
            remove_synector_task(task_name)?;
        }
    }

    let cmd = format!("\"{}\" --daemon", exe_path.to_string_lossy());
    let result = run_schtasks(&[
        "/create",
        "/tn",
        active_name,
        "/tr",
        &cmd,
        "/sc",
        "onlogon",
        "/rl",
        "highest",
        "/f",
    ])?;
    if !result.success {
        return Err(schtasks_error(
            "Failed to create the Task Scheduler task",
            &result,
        ));
    }
    if !task_belongs_to_synector(active_name) {
        return Err(format!(
            "Task Scheduler accepted task '{active_name}' but it does not point at a Synector executable"
        ));
    }
    Ok(())
}

fn disable_autostart(app_name: &str) -> Result<(), String> {
    for task_name in [app_name, MASKED_TASK_NAME, LEGACY_MASKED_TASK_NAME] {
        remove_synector_task(task_name)?;
    }
    if is_autorun_enabled_uncached(app_name) {
        return Err(
            "Autostart is still enabled after deleting the Synector scheduled tasks".to_string(),
        );
    }
    Ok(())
}

pub fn set_autorun_with_mask(
    app_name: &str,
    enable: bool,
    mask_daemon_process: bool,
) -> Result<(), String> {
    let active_name = if mask_daemon_process {
        MASKED_TASK_NAME
    } else {
        app_name
    };
    let result = if enable {
        enable_autostart(active_name, mask_daemon_process)
    } else {
        disable_autostart(app_name)
    };
    invalidate_autorun_status_cache();
    result
}

pub fn set_autorun(app_name: &str, enable: bool) -> Result<(), String> {
    let mask = crate::config::Settings::read().mask_daemon_process;
    set_autorun_with_mask(app_name, enable, mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_task_command_strips_quotes() {
        let xml = r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Actions Context="Author">
    <Exec>
      <Command>"C:\Program Files\Synector\Synector.exe"</Command>
      <Arguments>--daemon</Arguments>
    </Exec>
  </Actions>
</Task>"#;
        assert_eq!(
            parse_task_command(xml).as_deref(),
            Some("C:\\Program Files\\Synector\\Synector.exe")
        );
    }

    #[test]
    fn test_parse_task_command_unescapes_entities() {
        assert_eq!(
            parse_task_command("<Command>C:\\Tools &amp; Stuff\\Synector.exe</Command>").as_deref(),
            Some("C:\\Tools & Stuff\\Synector.exe")
        );
        assert_eq!(parse_task_command("<Task></Task>"), None);
        assert_eq!(parse_task_command("<Command>   </Command>"), None);
    }

    #[test]
    fn test_paths_equal_normalizes_case_and_separators() {
        assert!(paths_equal(
            Path::new("C:\\Program Files\\Synector\\Synector.exe"),
            Path::new("c:/program files/synector/synector.exe")
        ));
        assert!(paths_equal(Path::new("C:\\Synector\\"), Path::new("C:\\Synector")));
        assert!(!paths_equal(
            Path::new("C:\\Tools\\Synector.exe"),
            Path::new("C:\\Apps\\Synector.exe")
        ));
        assert!(!paths_equal(Path::new(""), Path::new("")));
    }

    #[test]
    fn test_decode_schtasks_output_utf8_and_utf16() {
        assert!(decode_schtasks_output(b"<?xml version=\"1.0\"?><Task/>").contains("<Task/>"));

        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "<Task/>".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        assert!(decode_schtasks_output(&utf16).contains("<Task/>"));

        // `schtasks` may emit UTF-16LE without a BOM when its output is redirected.
        let mut utf16_no_bom = Vec::new();
        for unit in "<Task/>".encode_utf16() {
            utf16_no_bom.extend_from_slice(&unit.to_le_bytes());
        }
        assert!(decode_schtasks_output(&utf16_no_bom).contains("<Task/>"));
    }

    #[test]
    fn test_masked_task_name_is_namespaced() {
        assert!(MASKED_TASK_NAME.contains("Synector"));
        assert_ne!(MASKED_TASK_NAME, LEGACY_MASKED_TASK_NAME);
        assert!(!MASKED_TASK_NAME.eq_ignore_ascii_case("WindowsServiceHost"));
    }

    #[test]
    fn test_command_belongs_to_synector() {
        let current = Path::new("C:\\Program Files\\Synector\\Synector.exe");
        let masked = Path::new("C:\\Users\\u\\AppData\\Local\\Synector\\bin\\WindowsServiceHost.exe");

        assert!(command_belongs_to_synector(current, current, masked));
        assert!(command_belongs_to_synector(
            Path::new("D:\\Portable\\Synector.exe"),
            current,
            masked
        ));
        assert!(command_belongs_to_synector(
            Path::new("C:\\Users\\u\\AppData\\Local\\Synector\\bin\\windowsservicehost.EXE"),
            current,
            masked
        ));
        assert!(!command_belongs_to_synector(
            Path::new("C:\\Windows\\System32\\cmd.exe"),
            current,
            masked
        ));
        assert!(!command_belongs_to_synector(Path::new(""), current, masked));
    }

    #[test]
    fn test_schtasks_error_includes_stderr() {
        let failed = SchtasksResult {
            success: false,
            stdout: String::new(),
            stderr: "ERROR: Access is denied.".to_string(),
        };
        assert_eq!(
            schtasks_error("Failed to create the Task Scheduler task", &failed),
            "Failed to create the Task Scheduler task: ERROR: Access is denied."
        );

        let silent = SchtasksResult {
            success: false,
            stdout: String::new(),
            stderr: String::new(),
        };
        assert!(schtasks_error("Failed to delete task", &silent).contains("no details"));
    }
}
