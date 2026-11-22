//! Share code format: the `SYN1:` prefix, compression dictionary, bounds and typed errors.
//!
//! These values are part of the wire contract and must stay byte-identical.

use std::fmt;

/// Prefix of a shareable automation code: `SYN1:` marks Synector share format version 1.
pub const SHARE_PREFIX: &str = "SYN1:";

/// Payload schema version carried inside the code.
pub const SHARE_VERSION: u32 = 1;

/// Brotli quality setting for export: 11 produces the smallest payload and takes the longest.
pub(super) const SHARE_QUALITY: i32 = 11;

/// Brotli custom dictionary: JSON keys and enum values that dominate share payloads.
///
/// It seeds the compressor's history, which measurably shrinks short codes (a few hundred
/// bytes each). This dictionary is part of the `SYN1` format: changing or removing it would
/// make existing codes undecodable, so it may only be replaced together with a format bump.
pub(super) const SHARE_DICTIONARY: &[u8] = br#"
"$action_type" "$trigger_type" "type" "id" "name" "description" "is_enabled"
"cooldown_seconds" "execution_count" "last_executed_at" "max_executions" "stop_on_error"
"trigger" "conditions" "actions" "on_exit_actions" "custom_icon_path" "auto_process_icon"
"tags" "version" "app_version" "created_at" "rules" "overlays" "macro_presets" "operation"
"start_process" "file_path" "arguments" "working_directory" "run_as_admin"
"only_if_not_running" "run_hidden" "close_on_exit" "kill_process" "process_name"
"kill_tree" "force" "notification" "title" "message" "http_webhook" "url" "method"
"headers" "payload" "execute_script" "interpreter" "script" "run_in_window" "wait" "mode"
"seconds" "condition" "timeout_sec" "timeout_policy" "check" "if_else" "then_actions"
"else_if_branches" "else_actions" "manual" "process" "match_mode" "single_instance"
"hotkey" "key_combination" "hold_mode" "repeat_while_held" "schedule" "schedule_type"
"interval_sec" "daily_time" "days_of_week" "day_of_month" "delay_sec" "target_datetime"
"webhook" "endpoint_path" "file_watcher" "folder_path" "filter_extension" "on_created"
"on_modified" "on_deleted" "system" "event_type" "idle_duration_sec" "device_id_filter"
"window_focus" "on_focus_gained" "on_focus_lost" "resource" "cpu_percent" "ram_mb"
"duration_sec" "macro" "preset_id" "events" "repeat" "speed" "infinite" "overlay"
"overlay_id" "custom_overlay" "override_text" "duration_ms" "run_as_admin" "priority"
"affinity_mask" "trim_memory" "play_sound" "play_beep" "beep_frequency" "beep_duration_ms"
"firewall" "power_plan" "display" "audio_control" "volume_level" "discord_rpc_set"
"archive" "file_system" "create_parents" "overwrite" "source_path" "destination_path"
"exact" "wildcard" "regex" "snake_case" "timer" "until_condition" "abort" "continue"
"foreground" "background" "idle" "user_active" "session_lock" "session_unlock" "sleep"
"resume" "power_source" "monitor" "network" "devices" "app_startup" "app_shutdown"
"interval" "daily" "one_shot" "monthly" "weekday" "weekend" "preset_id" "macro_presets"
"$action_type":"start_process" "$action_type":"notification" "$action_type":"wait"
"$action_type":"execute_script" "$action_type":"overlay" "$action_type":"macro"
"$trigger_type":"manual" "$trigger_type":"process" "$trigger_type":"hotkey"
"$trigger_type":"schedule" "$trigger_type":"file_watcher" "$trigger_type":"system"
"$trigger_type":"webhook" "$trigger_type":"window_focus" "$trigger_type":"resource"
"name":"description":"id":"type":"data":
"#;

/// Bytes of SHA-256 kept in front of the compressed payload.
pub(super) const CHECKSUM_LEN: usize = 8;

/// Largest accepted pasted code, in bytes of text (prefix included).
pub const MAX_CODE_CHARS: usize = 4 * 1024 * 1024;

/// Largest decompressed payload accepted from a share code.
pub const MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;

/// Largest number of rules in one shared payload or one import.
pub const MAX_SHARE_RULES: usize = 500;

/// Largest number of bundled overlays in one shared payload or one import.
///
/// Overlays and presets are persisted to `library.json` and rendered by the GUI, so they need
/// their own bound: the payload size limit alone would still allow thousands of tiny entries.
pub const MAX_SHARE_OVERLAYS: usize = 500;

/// Largest number of bundled macro presets in one shared payload or one import.
pub const MAX_SHARE_PRESETS: usize = 500;

/// Non-blocking notice about what a share code does not carry or what it can do.
///
/// The GUI renders these localized; the enum stays stable and language-neutral.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ShareWarning {
    /// A rule references a macro preset that is missing from both the code and the library.
    MissingMacroPreset(String),
    /// A rule references an overlay that is missing from both the code and the library.
    MissingOverlay(String),
    /// A rule runs scripts (PowerShell / cmd / Lua).
    SensitiveScript,
    /// A rule starts a process elevated.
    SensitiveAdminProcess,
    /// A rule performs an HTTP webhook request (headers/payload may contain secrets).
    SensitiveWebhook,
    /// A sender-local custom icon path is not carried by the code.
    CustomIconDropped,
    /// Absolute image or button icon paths inside an overlay are sender-local and were dropped.
    OverlayAssetDropped,
    /// An overlay click action or toggle binding points at a rule that is neither part of the
    /// code nor installed locally.
    MissingRule(String),
}

/// A code that could not be decoded into rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareError {
    EmptyInput,
    /// A pre-`SYN1` code; the old format is intentionally no longer decoded.
    LegacyFormat,
    /// Encoded data without the `SYN1:` marker.
    MissingPrefix,
    CodeTooLong,
    InvalidBase64,
    /// The compressed stream is broken or truncated.
    CorruptedPayload,
    /// The SHA-256 prefix does not match the decompressed payload.
    ChecksumMismatch,
    /// Decompressed payload exceeds [`MAX_PAYLOAD_BYTES`].
    PayloadTooLarge,
    /// Too many rules for one code/import.
    TooManyRules,
    /// Too many bundled overlays or macro presets for one code/import.
    TooManyLibraryItems,
    /// No rules, overlays or presets inside.
    EmptyPayload,
    /// JSON shape is not a share payload.
    InvalidPayload,
    /// The code was produced by a newer, unsupported Synector format.
    UnsupportedVersion(u32),
}

impl fmt::Display for ShareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(error_text(self))
    }
}

impl std::error::Error for ShareError {}

/// English fallback text; user-facing surfaces localize through [`ShareError`] variants.
pub fn error_text(error: &ShareError) -> &'static str {
    match error {
        ShareError::EmptyInput => "input is empty",
        ShareError::LegacyFormat => "legacy PH1 share codes are no longer supported",
        ShareError::MissingPrefix => "share code does not start with SYN1:",
        ShareError::CodeTooLong => "share code is too long",
        ShareError::InvalidBase64 => "share code is not valid base64",
        ShareError::CorruptedPayload => "share code is corrupted or truncated",
        ShareError::ChecksumMismatch => "share code failed its integrity check",
        ShareError::PayloadTooLarge => "share code expands to too much data",
        ShareError::TooManyRules => "share code contains too many automations",
        ShareError::TooManyLibraryItems => "share code contains too many library items",
        ShareError::EmptyPayload => "share code contains no automations",
        ShareError::InvalidPayload => "share code does not contain valid automation data",
        ShareError::UnsupportedVersion(_) => "share code was created by a newer Synector version",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::models::{OverlayDefinition, Rule};
    use crate::sharing::payload::fixtures::preset;
    use crate::sharing::{compress_and_encode, decode_and_decompress, prepare_import, SharePayload};

    #[test]
    fn test_limits_reject_huge_inputs() {
        assert_eq!(
            decode_and_decompress(&"A".repeat(MAX_CODE_CHARS + 1)).unwrap_err(),
            ShareError::CodeTooLong
        );
        assert_eq!(
            decode_and_decompress(&format!("{SHARE_PREFIX}!!")).unwrap_err(),
            ShareError::InvalidBase64
        );
        let mut too_many = SharePayload::rules_only(
            (0..MAX_SHARE_RULES + 1)
                .map(|i| Rule {
                    id: format!("r{i}"),
                    ..Default::default()
                })
                .collect(),
        );
        too_many.version = SHARE_VERSION;
        assert_eq!(
            compress_and_encode(&too_many).unwrap_err(),
            ShareError::TooManyRules
        );
    }

    #[test]
    fn test_typed_errors_have_stable_text() {
        assert_eq!(ShareError::EmptyInput.to_string(), error_text(&ShareError::EmptyInput));
        assert!(ShareError::LegacyFormat.to_string().contains("PH1"));
        assert_eq!(
            ShareError::UnsupportedVersion(7),
            ShareError::UnsupportedVersion(7)
        );
    }

    #[test]
    fn test_dictionary_is_stable_and_non_empty() {
        // The dictionary is part of the SYN1 format: replacing it would break every existing
        // code, so its size is pinned here as a reminder to bump the format instead.
        assert!(SHARE_DICTIONARY.len() > 512);
        assert!(SHARE_DICTIONARY.len() < 8 * 1024);
        assert!(SHARE_DICTIONARY.windows(7).any(|window| window == b"actions"));
    }

    #[test]
    fn test_share_payload_bounds_library_items() {
        let rule = Rule {
            id: "bounded_rule".into(),
            name: "Bounded".into(),
            ..Default::default()
        };
        let mut overlays = SharePayload::rules_only(vec![rule.clone()]);
        overlays.overlays = (0..=MAX_SHARE_OVERLAYS)
            .map(|index| {
                let mut item = OverlayDefinition::default_new_overlay();
                item.id = format!("overlay_{index}");
                item
            })
            .collect();
        assert_eq!(
            compress_and_encode(&overlays).unwrap_err(),
            ShareError::TooManyLibraryItems
        );
        let raw = serde_json::to_string(&overlays).expect("serialize");
        assert!(raw.len() < MAX_PAYLOAD_BYTES);
        assert_eq!(
            decode_and_decompress(&raw).unwrap_err(),
            ShareError::TooManyLibraryItems
        );
        assert_eq!(
            prepare_import(overlays, &[], &[], &[], false).unwrap_err(),
            ShareError::TooManyLibraryItems
        );

        let mut presets = SharePayload::rules_only(vec![rule]);
        presets.macro_presets = (0..=MAX_SHARE_PRESETS)
            .map(|index| preset(&format!("preset_{index}")))
            .collect();
        assert_eq!(
            compress_and_encode(&presets).unwrap_err(),
            ShareError::TooManyLibraryItems
        );
        assert_eq!(
            prepare_import(presets, &[], &[], &[], false).unwrap_err(),
            ShareError::TooManyLibraryItems
        );
    }
}
