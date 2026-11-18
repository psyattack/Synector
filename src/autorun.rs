pub const APP_NAME: &str = "Synector";

#[cfg(windows)]
fn run_schtasks(args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut cmd = std::process::Command::new("schtasks");
    cmd.args(args);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

#[cfg(windows)]
pub fn is_autorun_enabled(app_name: &str) -> bool {
    run_schtasks(&["/query", "/tn", app_name]) || run_schtasks(&["/query", "/tn", "WindowsServiceHost"])
}

/// Runs `schtasks` and returns its decoded standard output. Used to read a task definition back.
#[cfg(windows)]
fn run_schtasks_output(args: &[&str]) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut cmd = std::process::Command::new("schtasks");
    cmd.args(args);
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(decode_task_xml(&output.stdout))
}

/// Decodes `schtasks /xml` output, which is UTF-16LE on current Windows versions but is written
/// as UTF-8 when redirected in some builds.
fn decode_task_xml(bytes: &[u8]) -> String {
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

#[cfg(windows)]
fn scheduled_task_command(task_name: &str) -> Option<std::path::PathBuf> {
    let xml = run_schtasks_output(&["/query", "/tn", task_name, "/xml"])?;
    parse_task_command(&xml).map(std::path::PathBuf::from)
}

fn paths_equal(a: &std::path::Path, b: &std::path::Path) -> bool {
    fn normalize(path: &std::path::Path) -> String {
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    }
    !a.as_os_str().is_empty() && !b.as_os_str().is_empty() && normalize(a) == normalize(b)
}

/// Repoints the autostart task at the current executable after Synector moved (for example when
/// a portable copy was replaced by the installer). The task's presence stays the source of truth
/// for whether autostart is enabled; it is only rewritten when its target is stale.
#[cfg(windows)]
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
        "WindowsServiceHost"
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

#[cfg(not(windows))]
pub fn ensure_autorun_target_current(_mask_daemon_process: bool) {}

#[cfg(windows)]
pub fn set_autorun_with_mask(app_name: &str, enable: bool, mask_daemon_process: bool) -> Result<(), String> {
    let active_name = if mask_daemon_process { "WindowsServiceHost" } else { app_name };
    let alt_name = if mask_daemon_process { app_name } else { "WindowsServiceHost" };

    if enable {
        let exe_path = if mask_daemon_process {
            crate::daemon::pe_mask::prepare_masked_daemon().unwrap_or_else(|_| std::env::current_exe().unwrap_or_default())
        } else {
            std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?
        };
        let cmd = format!("\"{}\" --daemon", exe_path.to_string_lossy());

        // Remove alternate task if switching mask mode
        let _ = run_schtasks(&["/delete", "/tn", alt_name, "/f"]);

        // Create Windows Task Scheduler task with highest privileges (no UAC prompt on logon)
        let created = run_schtasks(&[
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
        ]);

        if created {
            Ok(())
        } else {
            Err("Failed to create Task Scheduler task".to_string())
        }
    } else {
        let _ = run_schtasks(&["/delete", "/tn", active_name, "/f"]);
        let _ = run_schtasks(&["/delete", "/tn", alt_name, "/f"]);
        Ok(())
    }
}

pub fn set_autorun(app_name: &str, enable: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mask = crate::config::Settings::read().mask_daemon_process;
        set_autorun_with_mask(app_name, enable, mask)
    }
    #[cfg(not(windows))]
    {
        let _ = app_name;
        let _ = enable;
        Ok(())
    }
}

#[cfg(not(windows))]
pub fn is_autorun_enabled(_app_name: &str) -> bool {
    false
}

#[cfg(not(windows))]
pub fn set_autorun_with_mask(_app_name: &str, _enable: bool, _mask_daemon_process: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
    fn test_decode_task_xml_utf8_and_utf16() {
        assert!(decode_task_xml(b"<?xml version=\"1.0\"?><Task/>").contains("<Task/>"));

        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "<Task/>".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        assert!(decode_task_xml(&utf16).contains("<Task/>"));

        // `schtasks` may emit UTF-16LE without a BOM when its output is redirected.
        let mut utf16_no_bom = Vec::new();
        for unit in "<Task/>".encode_utf16() {
            utf16_no_bom.extend_from_slice(&unit.to_le_bytes());
        }
        assert!(decode_task_xml(&utf16_no_bom).contains("<Task/>"));
    }
}
