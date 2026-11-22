use std::path::{Path, PathBuf};
use crate::models::{Action, Rule, Trigger};
use crate::win_com::{Apartment, ComApartment};
use windows::core::{HSTRING, Interface, PCWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

/// Upper bound for a shortcut file stem derived from a user string.
const MAX_SHORTCUT_STEM_CHARS: usize = 80;
/// Upper bound for the rule-id part of a collision-free shortcut name.
const MAX_RULE_ID_STEM_CHARS: usize = 40;
const MAX_SHORTCUT_SUFFIX: u32 = 99;
const SHORTCUT_STRING_BUFFER: usize = 8192;
const FALLBACK_STEM: &str = "Synector_Rule";

/// Creates a Windows shortcut (.lnk file) targeting Synector with `--run-rule <rule.id>`.
///
/// The shortcut is written through `IShellLink`/`IPersistFile`, so the rule id, name and
/// description never pass through a command interpreter. An existing `.lnk` is reused only when it
/// already points at Synector and carries exactly the current `--run-rule` arguments for this rule;
/// otherwise a rule-id-suffixed name is used so a foreign file (for example `Chrome.lnk` on the
/// Desktop, or a `.lnk` left by an older argument format) is never overwritten.
pub fn create_rule_shortcut(rule: &Rule, destination_folder: &Path) -> Result<PathBuf, String> {
    if !destination_folder.exists() {
        std::fs::create_dir_all(destination_folder)
            .map_err(|e| format!("Failed to create destination folder '{}': {e}", destination_folder.display()))?;
    }

    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to resolve Synector executable path: {e}"))?;
    let base_name = sanitize_file_stem(&rule.name, MAX_SHORTCUT_STEM_CHARS);
    let shortcut_path = choose_shortcut_path(destination_folder, &base_name, &rule.id, &current_exe)?;

    let arguments = shortcut_arguments(&rule.id);
    let working_dir = current_exe.parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let icon_location = resolve_rule_icon_path(rule, &current_exe.to_string_lossy());
    let description = if !rule.description.trim().is_empty() {
        rule.description.clone()
    } else {
        format!("Synector Automation: {}", rule.name)
    };

    write_shortcut(
        &shortcut_path,
        &current_exe,
        &arguments,
        &working_dir,
        &icon_location,
        &description,
    )?;

    if shortcut_path.exists() {
        Ok(shortcut_path)
    } else {
        Err(format!("Shortcut file was not created at '{}'", shortcut_path.display()))
    }
}

/// The arguments a rule shortcut launches Synector with.
fn shortcut_arguments(rule_id: &str) -> String {
    format!(
        "{} {}",
        crate::arg_spellings::RUN_RULE,
        crate::win_args::quote_windows_argument(rule_id)
    )
}

fn sanitize_file_stem(raw: &str, max_chars: usize) -> String {
    let sanitized: String = raw
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            other if other.is_control() => '_',
            other => other,
        })
        .collect();
    let sanitized = sanitized.trim().trim_end_matches('.').trim();
    let limited: String = sanitized.chars().take(max_chars).collect();
    let limited = limited.trim_end_matches('.').trim();
    if limited.is_empty() {
        FALLBACK_STEM.to_string()
    } else {
        limited.to_string()
    }
}

fn choose_shortcut_path(
    destination_folder: &Path,
    base_name: &str,
    rule_id: &str,
    current_exe: &Path,
) -> Result<PathBuf, String> {
    let canonical = destination_folder.join(format!("{base_name}.lnk"));
    if !canonical.exists() || shortcut_belongs_to_rule(&canonical, current_exe, rule_id) {
        return Ok(canonical);
    }

    // The plain name is taken by a foreign shortcut or by another rule: keep it and derive a
    // collision-free name from the rule id instead of overwriting anything.
    let id_stem = sanitize_file_stem(rule_id, MAX_RULE_ID_STEM_CHARS);
    let unique = destination_folder.join(format!("{base_name} - {id_stem}.lnk"));
    if !unique.exists() || shortcut_belongs_to_rule(&unique, current_exe, rule_id) {
        return Ok(unique);
    }

    for suffix in 2..=MAX_SHORTCUT_SUFFIX {
        let candidate = destination_folder.join(format!("{base_name} - {id_stem} ({suffix}).lnk"));
        if !candidate.exists() || shortcut_belongs_to_rule(&candidate, current_exe, rule_id) {
            return Ok(candidate);
        }
    }

    Err(format!(
        "Could not find a free shortcut name for rule '{rule_id}' in '{}'",
        destination_folder.display()
    ))
}

fn shortcut_belongs_to_rule(path: &Path, current_exe: &Path, rule_id: &str) -> bool {
    let Some(info) = read_shortcut(path) else {
        return false;
    };
    path_matches(&info.target, current_exe) && info.arguments == shortcut_arguments(rule_id)
}

fn path_matches(a: &Path, b: &Path) -> bool {
    fn normalize(path: &Path) -> String {
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    }
    !a.as_os_str().is_empty() && !b.as_os_str().is_empty() && normalize(a) == normalize(b)
}

fn create_shell_link() -> Result<IShellLinkW, String> {
    unsafe {
        CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("Failed to create the shell link COM object: {e}"))
    }
}

fn write_shortcut(
    shortcut_path: &Path,
    target: &Path,
    arguments: &str,
    working_dir: &str,
    icon_location: &str,
    description: &str,
) -> Result<(), String> {
    let _apartment = ComApartment::try_init(Apartment::SingleThreaded)?;
    unsafe {
        let link = create_shell_link()?;

        let target_wide = HSTRING::from(target.to_string_lossy().as_ref());
        link.SetPath(PCWSTR(target_wide.as_ptr()))
            .map_err(|e| format!("Failed to set the shortcut target: {e}"))?;
        let arguments_wide = HSTRING::from(arguments);
        link.SetArguments(PCWSTR(arguments_wide.as_ptr()))
            .map_err(|e| format!("Failed to set the shortcut arguments: {e}"))?;
        let working_dir_wide = HSTRING::from(working_dir);
        link.SetWorkingDirectory(PCWSTR(working_dir_wide.as_ptr()))
            .map_err(|e| format!("Failed to set the shortcut working directory: {e}"))?;
        let icon_wide = HSTRING::from(icon_location);
        link.SetIconLocation(PCWSTR(icon_wide.as_ptr()), 0)
            .map_err(|e| format!("Failed to set the shortcut icon: {e}"))?;
        let description_wide = HSTRING::from(description);
        link.SetDescription(PCWSTR(description_wide.as_ptr()))
            .map_err(|e| format!("Failed to set the shortcut description: {e}"))?;

        let persist: IPersistFile = link
            .cast()
            .map_err(|e| format!("Failed to access the shortcut file interface: {e}"))?;
        let path_wide = HSTRING::from(shortcut_path.to_string_lossy().as_ref());
        persist
            .Save(PCWSTR(path_wide.as_ptr()), true)
            .map_err(|e| format!("Failed to save the shortcut file: {e}"))?;
    }
    Ok(())
}

struct ShortcutInfo {
    target: PathBuf,
    arguments: String,
}

fn read_shortcut(path: &Path) -> Option<ShortcutInfo> {
    let _apartment = ComApartment::try_init(Apartment::SingleThreaded).ok()?;
    unsafe {
        let link = create_shell_link().ok()?;
        let persist: IPersistFile = link.cast().ok()?;
        let path_wide = HSTRING::from(path.to_string_lossy().as_ref());
        persist.Load(PCWSTR(path_wide.as_ptr()), STGM_READ).ok()?;

        let mut target_buffer = vec![0u16; SHORTCUT_STRING_BUFFER];
        link.GetPath(&mut target_buffer, std::ptr::null_mut(), 0).ok()?;
        let mut arguments_buffer = vec![0u16; SHORTCUT_STRING_BUFFER];
        link.GetArguments(&mut arguments_buffer).ok()?;

        Some(ShortcutInfo {
            target: PathBuf::from(wide_buffer_to_string(&target_buffer)),
            arguments: wide_buffer_to_string(&arguments_buffer).trim().to_string(),
        })
    }
}

fn wide_buffer_to_string(buffer: &[u16]) -> String {
    let len = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
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

    fn test_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "synector_sc_test_{}_{}",
            std::process::id(),
            label
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn lnk_files(dir: &Path) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().map(|ext| ext.eq_ignore_ascii_case("lnk")).unwrap_or(false))
            .collect();
        files.sort();
        files
    }

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
    fn test_sanitize_file_stem_replaces_reserved_characters() {
        assert_eq!(sanitize_file_stem("My Awesome / Rule", 80), "My Awesome _ Rule");
        assert_eq!(sanitize_file_stem("trailing...", 80), "trailing");
        assert_eq!(sanitize_file_stem("   ", 80), FALLBACK_STEM);
        assert_eq!(sanitize_file_stem("0123456789", 4), "0123");
    }

    #[test]
    fn test_shortcut_creation_in_temp_dir() {
        let temp_dir = test_dir("create");
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

    #[test]
    fn test_shortcut_stores_rule_id_verbatim() {
        // U17-001 regression: the id used to be interpolated into a single-quoted PowerShell
        // literal, so an apostrophe injected commands into the elevated GUI. It must now be
        // written as data, and the argument must survive quoting intact.
        let temp_dir = test_dir("verbatim");
        let rule_id = "rule 'quoted'; Start-Process calc; \"double\"; C:\\path with space\\";
        let rule = Rule {
            id: rule_id.into(),
            name: "My 'Rule' \"Quoted\" / name".into(),
            ..Default::default()
        };
        let path = create_rule_shortcut(&rule, &temp_dir).expect("shortcut creation must succeed");

        let info = read_shortcut(&path).expect("created shortcut must be readable through IShellLink");
        let current_exe = std::env::current_exe().expect("current exe must resolve");
        assert!(
            path_matches(&info.target, &current_exe),
            "shortcut must target Synector: {}",
            info.target.display()
        );
        assert_eq!(
            info.arguments,
            shortcut_arguments(rule_id),
            "rule id must be stored through the argv quoting helper, never executed"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_shortcut_creation_keeps_foreign_file() {
        let temp_dir = test_dir("foreign");
        std::fs::create_dir_all(&temp_dir).unwrap();
        let foreign = temp_dir.join("Chrome.lnk");
        std::fs::write(&foreign, b"not a shortcut").unwrap();

        let rule = Rule {
            id: "rule_chrome".into(),
            name: "Chrome".into(),
            ..Default::default()
        };
        let created = create_rule_shortcut(&rule, &temp_dir).expect("shortcut creation must succeed");

        assert_ne!(created, foreign, "the foreign shortcut path must not be reused");
        assert_eq!(std::fs::read(&foreign).unwrap(), b"not a shortcut");
        assert!(created.exists());
        assert_eq!(lnk_files(&temp_dir).len(), 2);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_legacy_shortcut_is_not_overwritten() {
        // A `.lnk` written by the removed PowerShell implementation used `--run-rule "<id>"`.
        // That is not the current argument format, so the file is foreign: it must keep its
        // content and the correctly quoted shortcut must be created beside it.
        let temp_dir = test_dir("legacy");
        std::fs::create_dir_all(&temp_dir).unwrap();
        let rule = Rule {
            id: "rule_legacy".into(),
            name: "Legacy Rule".into(),
            ..Default::default()
        };
        let current_exe = std::env::current_exe().expect("current exe must resolve");
        let legacy_path = temp_dir.join("Legacy Rule.lnk");
        let legacy_arguments = format!("--run-rule \"{}\"", rule.id);
        let working_dir = current_exe
            .parent()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default();
        write_shortcut(
            &legacy_path,
            &current_exe,
            &legacy_arguments,
            &working_dir,
            &current_exe.to_string_lossy(),
            "Legacy shortcut",
        )
        .expect("legacy-looking shortcut must be writable");

        let created = create_rule_shortcut(&rule, &temp_dir).expect("shortcut creation must succeed");

        assert_ne!(created, legacy_path, "a legacy shortcut must not be overwritten");
        assert_eq!(
            created,
            temp_dir.join("Legacy Rule - rule_legacy.lnk"),
            "the existing name must stay taken and the rule id must seed the unique name"
        );
        assert_eq!(lnk_files(&temp_dir).len(), 2);
        let legacy = read_shortcut(&legacy_path).expect("legacy shortcut must still be readable");
        assert_eq!(legacy.arguments, legacy_arguments, "the legacy file must be untouched");
        assert!(path_matches(&legacy.target, &current_exe));
        let info = read_shortcut(&created).expect("created shortcut must be readable");
        assert_eq!(info.arguments, shortcut_arguments(&rule.id));
        assert!(path_matches(&info.target, &current_exe));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_shortcut_creation_is_idempotent_for_same_rule() {
        let temp_dir = test_dir("idempotent");
        let rule = Rule {
            id: "rule_same".into(),
            name: "Same Rule".into(),
            ..Default::default()
        };

        let first = create_rule_shortcut(&rule, &temp_dir).expect("first creation must succeed");
        let second = create_rule_shortcut(&rule, &temp_dir).expect("second creation must succeed");

        assert_eq!(first, second);
        assert_eq!(lnk_files(&temp_dir).len(), 1);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_two_rules_with_same_name_get_distinct_shortcuts() {
        let temp_dir = test_dir("same-name");
        let first_rule = Rule {
            id: "rule_a".into(),
            name: "Shared Name".into(),
            ..Default::default()
        };
        let second_rule = Rule {
            id: "rule_b".into(),
            name: "Shared Name".into(),
            ..Default::default()
        };

        let first = create_rule_shortcut(&first_rule, &temp_dir).expect("first creation must succeed");
        let second = create_rule_shortcut(&second_rule, &temp_dir).expect("second creation must succeed");

        assert_ne!(first, second, "rules sharing a name must not overwrite each other");
        assert!(first.exists() && second.exists());
        assert_eq!(lnk_files(&temp_dir).len(), 2);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
