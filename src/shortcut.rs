use std::path::{Path, PathBuf};
use std::process::Command;
use crate::models::{Action, Rule, Trigger};

/// Creates a Windows shortcut (.lnk file) targeting Synector with `--run-rule <rule.id>`.
pub fn create_rule_shortcut(rule: &Rule, destination_folder: &Path) -> Result<PathBuf, String> {
    if !destination_folder.exists() {
        std::fs::create_dir_all(destination_folder)
            .map_err(|e| format!("Failed to create destination folder '{}': {e}", destination_folder.display()))?;
    }

    let sanitized_name: String = rule.name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            other => other,
        })
        .collect();
    let sanitized_name = sanitized_name.trim();
    let base_name = if sanitized_name.is_empty() { "Synector_Rule" } else { sanitized_name };
    let shortcut_path = destination_folder.join(format!("{}.lnk", base_name));

    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to resolve Synector executable path: {e}"))?;
    let exe_path_str = current_exe.to_string_lossy().to_string();
    let working_dir = current_exe.parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let icon_location = resolve_rule_icon_path(rule, &exe_path_str);
    let description = if !rule.description.trim().is_empty() {
        rule.description.clone()
    } else {
        format!("Synector Automation: {}", rule.name)
    };

    #[cfg(windows)]
    {
        // PowerShell script using WScript.Shell COM interface
        let ps_script = format!(
            "$ws = New-Object -ComObject WScript.Shell; \
             $lnk = $ws.CreateShortcut('{}'); \
             $lnk.TargetPath = '{}'; \
             $lnk.Arguments = '--run-rule \"{}\"'; \
             $lnk.WorkingDirectory = '{}'; \
             $lnk.IconLocation = '{},0'; \
             $lnk.Description = '{}'; \
             $lnk.Save();",
            shortcut_path.to_string_lossy().replace('\'', "''"),
            exe_path_str.replace('\'', "''"),
            rule.id.replace('"', "\\\""),
            working_dir.replace('\'', "''"),
            icon_location.replace('\'', "''"),
            description.replace('\'', "''")
        );

        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &ps_script]);
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

        let output = cmd.output().map_err(|e| format!("Failed to spawn PowerShell: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("PowerShell failed to create shortcut: {}", stderr.trim()));
        }
    }

    #[cfg(not(windows))]
    {
        let _ = (icon_location, description);
        std::fs::write(&shortcut_path, format!("--run-rule {}", rule.id))
            .map_err(|e| format!("Failed to write shortcut file: {e}"))?;
    }

    if shortcut_path.exists() {
        Ok(shortcut_path)
    } else {
        Err(format!("Shortcut file was not created at '{}'", shortcut_path.display()))
    }
}

pub fn resolve_rule_icon_path(rule: &Rule, default_exe: &str) -> String {
    if let Some(custom) = &rule.custom_icon_path {
        if Path::new(custom).exists() {
            return custom.clone();
        }
    }

    if rule.auto_process_icon {
        if let Some(path) = find_rule_executable_path(rule) {
            return path;
        }
    }

    // Check StartProcess actions
    for action in &rule.actions {
        if let Action::StartProcess { file_path, .. } = action {
            if Path::new(file_path).exists() {
                return file_path.clone();
            }
        }
    }

    default_exe.to_string()
}

fn find_rule_executable_path(rule: &Rule) -> Option<String> {
    if let Some(Trigger::Process { process_name, .. }) = &rule.trigger {
        if let Some(path) = crate::daemon::watcher::ProcessWatcher::find_exe_path_by_name(process_name) {
            return Some(path);
        }
    }
    if let Some(Trigger::WindowFocus { process_name, .. }) = &rule.trigger {
        if let Some(path) = crate::daemon::watcher::ProcessWatcher::find_exe_path_by_name(process_name) {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_rule_icon_path_fallback() {
        let rule = Rule {
            name: "Test Rule".into(),
            ..Default::default()
        };
        let icon = resolve_rule_icon_path(&rule, "C:\\Synector.exe");
        assert_eq!(icon, "C:\\Synector.exe");
    }

    #[test]
    fn test_shortcut_creation_in_temp_dir() {
        let temp_dir = std::env::temp_dir().join(format!("synector_sc_test_{}", std::process::id()));
        let rule = Rule {
            id: "rule_test_sc_123".into(),
            name: "My Awesome Automation / Rule".into(),
            description: "Runs automatically".into(),
            ..Default::default()
        };
        let res = create_rule_shortcut(&rule, &temp_dir);
        assert!(res.is_ok(), "Shortcut creation must succeed: {:?}", res.err());
        let path = res.unwrap();
        assert!(path.exists(), "Shortcut file must exist on disk");
        assert!(path.to_string_lossy().ends_with(".lnk"));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
