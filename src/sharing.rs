//! Portable automation exchange: one compact text code carries rules together with the
//! library presets and overlays they reference.
//!
//! Wire format (`SYN1:` prefix):
//!
//! ```text
//! SYN1:base64url( sha256(payload_json)[..8] || brotli(payload_json) )
//! ```
//!
//! The 8-byte SHA-256 prefix catches truncated or altered codes (brotli itself has no
//! checksum), the decompressor is bounded so a malicious code cannot exhaust memory, and
//! imports never carry runtime counters or sender-local icon and image paths.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::prelude::*;
use brotli::enc::BrotliEncoderParams;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::models::{
    Action, InteractiveControlKind, MacroPreset, OverlayClickAction, OverlayDefinition,
    OverlayElement, Rule,
};

/// Prefix of a shareable automation code: `SYN1:` marks Synector share format version 1.
pub const SHARE_PREFIX: &str = "SYN1:";

/// Payload schema version carried inside the code.
pub const SHARE_VERSION: u32 = 1;

/// Brotli quality setting for export: 11 produces the smallest payload and takes the longest.
const SHARE_QUALITY: i32 = 11;

/// Brotli custom dictionary: JSON keys and enum values that dominate share payloads.
///
/// It seeds the compressor's history, which measurably shrinks short codes (a few hundred
/// bytes each). This dictionary is part of the `SYN1` format: changing or removing it would
/// make existing codes undecodable, so it may only be replaced together with a format bump.
const SHARE_DICTIONARY: &[u8] = br#"
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
const CHECKSUM_LEN: usize = 8;

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

// ---------------------------------------------------------------------------
// Payload
// ---------------------------------------------------------------------------

/// A decoded share code: the rules plus the library entries they reference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SharePayload {
    #[serde(default = "default_share_version")]
    pub version: u32,
    /// Synector version that produced the code, for diagnostics and previews.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub app_version: String,
    /// RFC 3339 creation timestamp of the code.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,
    pub rules: Vec<Rule>,
    /// Referenced overlay presets, included so the receiving machine can resolve them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<OverlayDefinition>,
    /// Referenced macro presets, included so the receiving machine can resolve them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macro_presets: Vec<MacroPreset>,
}

fn default_share_version() -> u32 {
    SHARE_VERSION
}

impl SharePayload {
    /// Wraps rules without library dependencies; used for the raw-JSON import fallback.
    pub fn rules_only(rules: Vec<Rule>) -> Self {
        Self {
            version: SHARE_VERSION,
            app_version: String::new(),
            created_at: String::new(),
            rules,
            overlays: Vec::new(),
            macro_presets: Vec::new(),
        }
    }
}

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

/// Result of applying a share payload to the local configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareImportSummary {
    pub imported: usize,
    pub skipped_duplicates: usize,
    pub overlays: usize,
    pub macro_presets: usize,
    pub enabled: bool,
}

/// Share payload resolved against the local library and rule set, ready to be applied.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PreparedShareImport {
    pub rules: Vec<Rule>,
    pub overlays: Vec<OverlayDefinition>,
    pub macro_presets: Vec<MacroPreset>,
    pub skipped_duplicates: usize,
    pub warnings: Vec<ShareWarning>,
    /// False when every incoming rule was already installed.
    pub has_new_content: bool,
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Compresses and encodes a payload as a `SYN1:` share code.
pub fn compress_and_encode(payload: &SharePayload) -> Result<String, ShareError> {
    validate_payload_limits(payload)?;
    let json_bytes = payload_json(payload)?;
    if json_bytes.len() > MAX_PAYLOAD_BYTES {
        return Err(ShareError::PayloadTooLarge);
    }
    if payload.rules.is_empty() && payload.overlays.is_empty() && payload.macro_presets.is_empty() {
        return Err(ShareError::EmptyPayload);
    }

    let checksum = Sha256::digest(&json_bytes);
    let compressed = brotli_compress_with_dictionary(&json_bytes)?;

    let mut framed = Vec::with_capacity(CHECKSUM_LEN + compressed.len());
    framed.extend_from_slice(&checksum[..CHECKSUM_LEN]);
    framed.extend_from_slice(&compressed);
    Ok(format!("{}{}", SHARE_PREFIX, BASE64_URL_SAFE_NO_PAD.encode(framed)))
}

/// Serializes the payload and drops fields that carry their serde default value.
///
/// Every stripped key is optional on deserialization, so the payload round-trips exactly while
/// the compressed code keeps only meaningful data. `Postcard`/binary encodings were rejected as
/// they lose the JSON compatibility and debuggability that share codes rely on.
fn payload_json(payload: &SharePayload) -> Result<Vec<u8>, ShareError> {
    let mut value = serde_json::to_value(payload).map_err(|_| ShareError::InvalidPayload)?;
    strip_default_share_fields(&mut value);
    serde_json::to_vec(&value).map_err(|_| ShareError::InvalidPayload)
}

fn strip_default_share_fields(value: &mut serde_json::Value) {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            for child in map.values_mut() {
                strip_default_share_fields(child);
            }
            map.retain(|key, child| !is_default_share_field(key, child));
        }
        Value::Array(items) => {
            for item in items {
                strip_default_share_fields(item);
            }
        }
        _ => {}
    }
}

/// Whitelist of keys whose serde default is known, so removing the default value is lossless.
fn is_default_share_field(key: &str, value: &serde_json::Value) -> bool {
    use serde_json::Value;
    let is_false = *value == Value::Bool(false);
    let is_true = *value == Value::Bool(true);
    let is_number = |number: f64| value.as_f64() == Some(number);
    let is_empty_array = value.as_array().map(|items| items.is_empty()).unwrap_or(false);
    let is_text = |text: &str| value.as_str() == Some(text);
    match key {
        // Action flags with `#[serde(default)]` (false).
        "run_as_admin"
        | "only_if_not_running"
        | "run_hidden"
        | "close_on_exit"
        | "kill_tree"
        | "force"
        | "play_beep"
        | "trim_memory"
        | "skip_if_focused"
        | "infinite"
        | "auto_clear_on_process_exit"
        | "organic"
        | "smooth"
        | "single_instance"
        | "hold_mode"
        | "repeat_while_held"
        | "on_focus_lost"
        | "on_modified"
        | "on_deleted" => is_false,
        // Action/trigger flags with `#[serde(default = "default_true")]`.
        "create_parents" | "overwrite" | "show_elapsed" | "on_focus_gained" | "on_created" => {
            is_true
        }
        // Numeric defaults (repeat 1, speed 1.0, wait 1s/30s, resource window 5s).
        "repeat" | "speed" | "seconds" => is_number(1.0),
        "timeout_sec" => is_number(30.0),
        "duration_sec" => is_number(5.0),
        // Trigger zeros.
        "interval_sec" => is_number(0.0),
        // Enum defaults.
        "method" => is_text("POST"),
        "timeout_policy" => is_text("abort"),
        "mode" => is_text("timer"),
        "operation" => is_text("run"),
        "match_mode" => is_text("exact"),
        // Empty collections that are optional on deserialization.
        "events" | "days_of_week" => is_empty_array,
        _ => false,
    }
}

/// Brotli-compresses the payload with the format dictionary seeded into the history.
fn brotli_compress_with_dictionary(json: &[u8]) -> Result<Vec<u8>, ShareError> {
    let params = BrotliEncoderParams {
        quality: SHARE_QUALITY,
        ..Default::default()
    };
    let mut input_buffer = [0u8; 4096];
    let mut output_buffer = [0u8; 4096];
    let mut compressed = Vec::new();
    let mut callback = |_context: &mut brotli::interface::PredictionModeContextMap<
        brotli::InputReferenceMut,
    >,
                        _commands: &mut [brotli::interface::StaticCommand],
                        _pair: brotli::interface::InputPair,
                        _alloc: &mut brotli::enc::StandardAlloc| {};
    let mut input = json;
    brotli::BrotliCompressCustomIoCustomDict(
        &mut brotli::IoReaderWrapper(&mut input),
        &mut brotli::IoWriterWrapper(&mut compressed),
        &mut input_buffer,
        &mut output_buffer,
        &params,
        brotli::enc::StandardAlloc::default(),
        &mut callback,
        SHARE_DICTIONARY,
        std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "unexpected EOF"),
    )
    .map_err(|_| ShareError::CorruptedPayload)?;
    Ok(compressed)
}

/// Decompresses a payload encoded with [`SHARE_DICTIONARY`]; bounded by [`MAX_PAYLOAD_BYTES`].
fn brotli_decompress_with_dictionary(compressed: &[u8]) -> Result<Vec<u8>, ShareError> {
    type HeapAllocU8 = brotli::HeapAlloc<u8>;
    let dictionary: <HeapAllocU8 as brotli::Allocator<u8>>::AllocatedMemory =
        SHARE_DICTIONARY.to_vec().into();
    let mut decoder = brotli::Decompressor::new_with_custom_dict(compressed, 4096, dictionary);
    let mut json_bytes = Vec::new();
    decoder
        .by_ref()
        .take((MAX_PAYLOAD_BYTES + 1) as u64)
        .read_to_end(&mut json_bytes)
        .map_err(|_| ShareError::CorruptedPayload)?;
    if json_bytes.len() > MAX_PAYLOAD_BYTES {
        return Err(ShareError::PayloadTooLarge);
    }
    Ok(json_bytes)
}

/// Builds the payload for the given rules and returns notices about dependencies and risks.
///
/// Sender-local data is stripped here: runtime counters never leave the machine, and custom
/// icon, overlay image and button icon paths (which point at the sender's disk) are dropped
/// with a warning.
pub fn build_payload(
    mut rules: Vec<Rule>,
    library_overlays: &[OverlayDefinition],
    library_presets: &[MacroPreset],
) -> (SharePayload, Vec<ShareWarning>) {
    let mut warnings = Vec::new();

    for rule in &mut rules {
        rule.clear_quota_runtime();
        if rule.custom_icon_path.take().is_some() {
            push_warning(&mut warnings, ShareWarning::CustomIconDropped);
        }
        if strip_rule_overlay_assets(rule) {
            push_warning(&mut warnings, ShareWarning::OverlayAssetDropped);
        }
    }

    let mut overlays = Vec::new();
    let mut macro_presets = Vec::new();
    let mut seen_overlays = HashSet::new();
    let mut seen_presets = HashSet::new();

    for rule in &rules {
        rule.visit_actions(&mut |action| match action {
            Action::Macro {
                preset_id: Some(preset_id),
                ..
            } => {
                if library_presets.iter().any(|p| &p.id == preset_id) {
                    if seen_presets.insert(preset_id.clone()) {
                        if let Some(preset) =
                            library_presets.iter().find(|p| &p.id == preset_id)
                        {
                            macro_presets.push(preset.clone());
                        }
                    }
                } else {
                    push_warning(
                        &mut warnings,
                        ShareWarning::MissingMacroPreset(preset_id.clone()),
                    );
                }
            }
            Action::Overlay {
                overlay_id,
                custom_overlay: None,
                ..
            } if !overlay_id.is_empty() => {
                if library_overlays.iter().any(|o| &o.id == overlay_id) {
                    if seen_overlays.insert(overlay_id.clone()) {
                        if let Some(overlay) =
                            library_overlays.iter().find(|o| &o.id == overlay_id)
                        {
                            let mut bundled = overlay.clone();
                            if strip_overlay_asset_paths(&mut bundled) {
                                push_warning(&mut warnings, ShareWarning::OverlayAssetDropped);
                            }
                            overlays.push(bundled);
                        }
                    }
                } else {
                    push_warning(&mut warnings, ShareWarning::MissingOverlay(overlay_id.clone()));
                }
            }
            _ => {}
        });
        collect_sensitive_warnings(rule, &mut warnings);
    }

    let payload = SharePayload {
        version: SHARE_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: chrono::Local::now().to_rfc3339(),
        rules,
        overlays,
        macro_presets,
    };
    (payload, warnings)
}

fn push_warning(warnings: &mut Vec<ShareWarning>, warning: ShareWarning) {
    if !warnings.contains(&warning) {
        warnings.push(warning);
    }
}

fn collect_sensitive_warnings(rule: &Rule, warnings: &mut Vec<ShareWarning>) {
    rule.visit_actions(&mut |action| match action {
        Action::ExecuteScript { .. } => {
            push_warning(warnings, ShareWarning::SensitiveScript);
        }
        Action::StartProcess {
            run_as_admin: true, ..
        } => {
            push_warning(warnings, ShareWarning::SensitiveAdminProcess);
        }
        Action::HttpWebhook { .. } => {
            push_warning(warnings, ShareWarning::SensitiveWebhook);
        }
        _ => {}
    });
}

// ---------------------------------------------------------------------------
// Sender-local asset paths
// ---------------------------------------------------------------------------

/// Drops every sender-local image and button icon path from one overlay definition.
///
/// Overlay images and button icons are picked from the sender's disk, so shipping the absolute
/// paths would leak their filesystem layout and install broken images on the receiving machine.
/// Returns true when at least one path was dropped.
fn strip_overlay_asset_paths(overlay: &mut OverlayDefinition) -> bool {
    let mut dropped = false;
    for element in &mut overlay.elements {
        match element {
            OverlayElement::Image { path, .. } => {
                if !path.trim().is_empty() {
                    path.clear();
                    dropped = true;
                }
            }
            OverlayElement::InteractiveControl {
                control: InteractiveControlKind::Button { icon, .. },
                ..
            } => {
                if icon.take().is_some_and(|path| !path.trim().is_empty()) {
                    dropped = true;
                }
            }
            _ => {}
        }
    }
    dropped
}

/// Drops sender-local asset paths from overlays embedded directly in a rule's actions.
fn strip_rule_overlay_assets(rule: &mut Rule) -> bool {
    let mut dropped = false;
    for action in rule.actions.iter_mut().chain(rule.on_exit_actions.iter_mut()) {
        dropped |= strip_action_overlay_assets(action);
    }
    dropped
}

fn strip_action_overlay_assets(action: &mut Action) -> bool {
    match action {
        Action::Overlay { custom_overlay, .. } => custom_overlay
            .as_deref_mut()
            .map(strip_overlay_asset_paths)
            .unwrap_or(false),
        Action::IfElse {
            then_actions,
            else_if_branches,
            else_actions,
            ..
        } => {
            let mut dropped = false;
            for action in then_actions {
                dropped |= strip_action_overlay_assets(action);
            }
            for branch in else_if_branches {
                for action in &mut branch.actions {
                    dropped |= strip_action_overlay_assets(action);
                }
            }
            for action in else_actions {
                dropped |= strip_action_overlay_assets(action);
            }
            dropped
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Sanitizes and decodes a pasted share code (or a raw JSON paste) into a payload.
///
/// Accepts the code wrapped in backticks/quotes and tolerates whitespace and line breaks
/// anywhere inside the encoded body, which is what chat clients insert when wrapping.
pub fn decode_and_decompress(raw_input: &str) -> Result<SharePayload, ShareError> {
    let clean = raw_input
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim();
    if clean.is_empty() {
        return Err(ShareError::EmptyInput);
    }
    if clean.len() > MAX_CODE_CHARS {
        return Err(ShareError::CodeTooLong);
    }

    if let Some(encoded) = clean.strip_prefix(SHARE_PREFIX) {
        return decode_code(encoded);
    }
    if clean.starts_with("PH1:") {
        return Err(ShareError::LegacyFormat);
    }
    // Plain JSON paste (for example a rule exported with --rules-export) stays supported.
    decode_raw_json(clean)
}

fn decode_code(encoded: &str) -> Result<SharePayload, ShareError> {
    let compressed = decode_base64(encoded)?;
    if compressed.len() <= CHECKSUM_LEN {
        return Err(ShareError::CorruptedPayload);
    }
    let (checksum, compressed_body) = compressed.split_at(CHECKSUM_LEN);

    let json_bytes = brotli_decompress_with_dictionary(compressed_body)?;

    let digest = Sha256::digest(&json_bytes);
    if !checksums_match(checksum, &digest) {
        return Err(ShareError::ChecksumMismatch);
    }

    let value: serde_json::Value =
        serde_json::from_slice(&json_bytes).map_err(|_| ShareError::InvalidPayload)?;
    validate_share_payload_value(&value)?;
    let payload: SharePayload =
        serde_json::from_value(value).map_err(|_| ShareError::InvalidPayload)?;
    validate_payload(payload)
}

/// Compares the truncated digest with an XOR accumulator instead of slice equality.
///
/// Slice equality stops at the first differing byte; the only integrity check in the format
/// must not reveal where a mismatch happened. The length check comes first because the digest
/// length is public.
fn checksums_match(checksum: &[u8], digest: &[u8]) -> bool {
    if checksum.len() != CHECKSUM_LEN || digest.len() < CHECKSUM_LEN {
        return false;
    }
    let mut difference = 0u8;
    for (expected, actual) in checksum.iter().zip(digest.iter()) {
        difference |= expected ^ actual;
    }
    difference == 0
}

fn decode_base64(encoded: &str) -> Result<Vec<u8>, ShareError> {
    let cleaned: String = encoded
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    if cleaned.is_empty() {
        return Err(ShareError::InvalidBase64);
    }
    // Tolerate both the URL-safe alphabet we emit and the standard one from older builds,
    // with or without trailing padding.
    let mut normalized = String::with_capacity(cleaned.len());
    for ch in cleaned.chars() {
        normalized.push(match ch {
            '-' => '+',
            '_' => '/',
            other => other,
        });
    }
    let unpadded = normalized.trim_end_matches('=');
    BASE64_STANDARD_NO_PAD
        .decode(unpadded)
        .map_err(|_| ShareError::InvalidBase64)
}

fn decode_raw_json(raw: &str) -> Result<SharePayload, ShareError> {
    if raw.len() > MAX_PAYLOAD_BYTES {
        return Err(ShareError::PayloadTooLarge);
    }
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|_| ShareError::InvalidPayload)?;
    let payload = match value {
        serde_json::Value::Object(object) if object.contains_key("rules") => {
            let value = serde_json::Value::Object(object);
            validate_share_payload_value(&value)?;
            serde_json::from_value::<SharePayload>(value).map_err(|_| ShareError::InvalidPayload)?
        }
        serde_json::Value::Object(_) => {
            validate_rule_value(&value)?;
            let rule: Rule = serde_json::from_value(value).map_err(|_| ShareError::InvalidPayload)?;
            SharePayload::rules_only(vec![rule])
        }
        serde_json::Value::Array(_) => {
            validate_rule_array(&value)?;
            let rules: Vec<Rule> =
                serde_json::from_value(value).map_err(|_| ShareError::InvalidPayload)?;
            SharePayload::rules_only(rules)
        }
        _ => return Err(ShareError::InvalidPayload),
    };
    validate_payload(payload)
}

/// Rejects unknown keys in a raw share payload before serde can drop them silently.
fn validate_share_payload_value(value: &serde_json::Value) -> Result<(), ShareError> {
    let Some(object) = value.as_object() else {
        return Err(ShareError::InvalidPayload);
    };
    const PAYLOAD_KEYS: &[&str] = &[
        "version",
        "app_version",
        "created_at",
        "rules",
        "overlays",
        "macro_presets",
    ];
    if object
        .keys()
        .any(|key| !PAYLOAD_KEYS.contains(&key.as_str()))
    {
        return Err(ShareError::InvalidPayload);
    }
    if let Some(rules) = object.get("rules") {
        validate_rule_array(rules)?;
    }
    Ok(())
}

fn validate_rule_array(value: &serde_json::Value) -> Result<(), ShareError> {
    let Some(rules) = value.as_array() else {
        return Ok(());
    };
    for rule in rules {
        validate_rule_value(rule)?;
    }
    Ok(())
}

fn validate_rule_value(value: &serde_json::Value) -> Result<(), ShareError> {
    if crate::models::validate_rule_keys(value).is_err() {
        return Err(ShareError::InvalidPayload);
    }
    // Parse once here so the expensive typed checks run before the caller parses the
    // payload; unknown keys are already gone, so the typed value is exact.
    let rule: Rule = serde_json::from_value(value.clone()).map_err(|_| ShareError::InvalidPayload)?;
    validate_rule_definition(&rule)
}

/// Value checks applied to every imported rule: tag colours, quota window bounds, schedule
/// fields, condition keys and wait durations.
fn validate_rule_definition(rule: &Rule) -> Result<(), ShareError> {
    let invalid = rule.tag_color_error().is_some()
        || rule
            .quota_period
            .and_then(crate::models::QuotaPeriod::validation_error)
            .is_some()
        || rule
            .trigger
            .as_ref()
            .is_some_and(|trigger| !trigger.schedule_errors().is_empty())
        || rule
            .conditions
            .iter()
            .any(|item| !item.condition.validation_errors().is_empty())
        || {
            let mut invalid = false;
            rule.visit_actions(&mut |action| {
                invalid |= !action.validation_errors().is_empty();
            });
            invalid
        };
    if invalid {
        Err(ShareError::InvalidPayload)
    } else {
        Ok(())
    }
}

fn validate_payload(payload: SharePayload) -> Result<SharePayload, ShareError> {
    if payload.version > SHARE_VERSION {
        return Err(ShareError::UnsupportedVersion(payload.version));
    }
    validate_payload_limits(&payload)?;
    if payload.rules.is_empty() && payload.overlays.is_empty() && payload.macro_presets.is_empty() {
        return Err(ShareError::EmptyPayload);
    }
    Ok(payload)
}

/// Count bounds shared by encoding and importing.
fn validate_payload_limits(payload: &SharePayload) -> Result<(), ShareError> {
    if payload.rules.len() > MAX_SHARE_RULES {
        return Err(ShareError::TooManyRules);
    }
    if payload.overlays.len() > MAX_SHARE_OVERLAYS
        || payload.macro_presets.len() > MAX_SHARE_PRESETS
    {
        return Err(ShareError::TooManyLibraryItems);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Import preparation
// ---------------------------------------------------------------------------

/// Resolves a payload against the local library and rule set.
///
/// New library entries keep their ids when there is no clash; a clashing id with different
/// content gets a fresh id and every rule reference is rewritten to match. An entry whose
/// content already exists locally under another id (a previous import of the same code) is
/// reused instead of duplicated. Runtime counters are reset, sender-local icon and overlay
/// image paths are dropped, rules are disabled unless `enable` is set, bundled overlays have
/// their rule click targets rewritten through the import id map, and already installed rules
/// are skipped as duplicates.
pub fn prepare_import(
    payload: SharePayload,
    existing_rules: &[Rule],
    existing_overlays: &[OverlayDefinition],
    existing_presets: &[MacroPreset],
    enable: bool,
) -> Result<PreparedShareImport, ShareError> {
    let mut payload = validate_payload(payload)?;

    let mut warnings = Vec::new();

    for rule in &mut payload.rules {
        rule.clear_quota_runtime();
        rule.is_enabled = enable;
        rule.normalize();
        if rule.custom_icon_path.take().is_some() {
            push_warning(&mut warnings, ShareWarning::CustomIconDropped);
        }
        if strip_rule_overlay_assets(rule) {
            push_warning(&mut warnings, ShareWarning::OverlayAssetDropped);
        }
    }
    for overlay in &mut payload.overlays {
        if strip_overlay_asset_paths(overlay) {
            push_warning(&mut warnings, ShareWarning::OverlayAssetDropped);
        }
    }

    // Library references are fingerprinted by content, not only by id: an id clash rewrites
    // the stored copy to a fresh id, so a re-import of the same code must still match it.
    let payload_overlay_hashes = overlay_content_index(&payload.overlays);
    let payload_preset_hashes = preset_content_index(&payload.macro_presets);
    let mut incoming_overlay_hashes = overlay_content_index(existing_overlays);
    incoming_overlay_hashes.extend(payload_overlay_hashes.clone());
    let mut incoming_preset_hashes = preset_content_index(existing_presets);
    incoming_preset_hashes.extend(payload_preset_hashes.clone());
    let mut local_overlay_hashes = payload_overlay_hashes;
    local_overlay_hashes.extend(overlay_content_index(existing_overlays));
    let mut local_preset_hashes = payload_preset_hashes;
    local_preset_hashes.extend(preset_content_index(existing_presets));

    // Rules come first: bundled overlays may point at these rules from their click actions,
    // so the final rule ids must be known before an overlay enters the library.
    let mut rules: Vec<Rule> = Vec::new();
    let mut skipped_duplicates = 0usize;
    let mut known_rule_ids: HashSet<String> = existing_rules.iter().map(|r| r.id.clone()).collect();
    let mut fingerprints: HashSet<String> = HashSet::new();
    let mut fingerprint_rule_ids: HashMap<String, String> = HashMap::new();
    for rule in existing_rules {
        let fingerprint = rule_fingerprint(rule, &local_overlay_hashes, &local_preset_hashes);
        fingerprints.insert(fingerprint.clone());
        fingerprint_rule_ids
            .entry(fingerprint)
            .or_insert_with(|| rule.id.clone());
    }
    let mut rule_id_map: HashMap<String, String> = HashMap::new();

    for mut rule in payload.rules {
        let original_id = rule.id.clone();
        let fingerprint =
            rule_fingerprint(&rule, &incoming_overlay_hashes, &incoming_preset_hashes);
        if fingerprints.contains(&fingerprint) {
            skipped_duplicates += 1;
            if !original_id.trim().is_empty() {
                if let Some(existing_id) = fingerprint_rule_ids.get(&fingerprint) {
                    rule_id_map.insert(original_id, existing_id.clone());
                }
            }
            continue;
        }
        if rule.id.trim().is_empty() || known_rule_ids.contains(&rule.id) {
            rule.id = fresh_id("rule");
            if !rule.name.ends_with("(Imported)") {
                rule.name = format!("{} (Imported)", rule.name);
            }
        }
        known_rule_ids.insert(rule.id.clone());
        fingerprints.insert(fingerprint.clone());
        fingerprint_rule_ids.insert(fingerprint, rule.id.clone());
        if !original_id.trim().is_empty() {
            rule_id_map.insert(original_id, rule.id.clone());
        }
        rules.push(rule);
    }

    let mut overlay_map: HashMap<String, String> = HashMap::new();
    let mut new_overlays: Vec<OverlayDefinition> = Vec::new();
    let mut known_overlay_ids: HashSet<String> = HashSet::new();
    for mut overlay in payload.overlays {
        if !known_overlay_ids.insert(overlay.id.clone()) {
            continue;
        }
        remap_overlay_rule_targets(&mut overlay, &rule_id_map, &known_rule_ids, &mut warnings);
        let original_id = overlay.id.clone();
        if let Some(existing) = existing_overlays.iter().find(|o| o.id == original_id) {
            if existing == &overlay {
                overlay_map.insert(original_id, existing.id.clone());
                continue;
            }
            if let Some(matching) = existing_overlays
                .iter()
                .find(|o| overlay_content_hash(o) == overlay_content_hash(&overlay))
            {
                overlay_map.insert(original_id, matching.id.clone());
                continue;
            }
            overlay.id = fresh_id("overlay");
        }
        overlay_map.insert(original_id, overlay.id.clone());
        new_overlays.push(overlay);
    }

    let mut preset_map: HashMap<String, String> = HashMap::new();
    let mut new_presets: Vec<MacroPreset> = Vec::new();
    let mut known_preset_ids: HashSet<String> = HashSet::new();
    for mut preset in payload.macro_presets {
        if !known_preset_ids.insert(preset.id.clone()) {
            continue;
        }
        let original_id = preset.id.clone();
        if let Some(existing) = existing_presets.iter().find(|p| p.id == original_id) {
            if existing == &preset {
                preset_map.insert(original_id, existing.id.clone());
                continue;
            }
            if let Some(matching) = existing_presets
                .iter()
                .find(|p| preset_content_hash(p) == preset_content_hash(&preset))
            {
                preset_map.insert(original_id, matching.id.clone());
                continue;
            }
            preset.id = fresh_id("preset");
        }
        preset_map.insert(original_id, preset.id.clone());
        new_presets.push(preset);
    }

    for rule in &mut rules {
        remap_library_refs(rule, &overlay_map, &preset_map);
    }

    let all_overlay_ids: HashSet<&str> = existing_overlays
        .iter()
        .map(|o| o.id.as_str())
        .chain(new_overlays.iter().map(|o| o.id.as_str()))
        .collect();
    let all_preset_ids: HashSet<&str> = existing_presets
        .iter()
        .map(|p| p.id.as_str())
        .chain(new_presets.iter().map(|p| p.id.as_str()))
        .collect();
    for rule in &rules {
        collect_sensitive_warnings(rule, &mut warnings);
        rule.visit_actions(&mut |action| match action {
            Action::Macro {
                preset_id: Some(preset_id),
                ..
            } if !all_preset_ids.contains(preset_id.as_str()) => {
                push_warning(
                    &mut warnings,
                    ShareWarning::MissingMacroPreset(preset_id.clone()),
                );
            }
            Action::Overlay {
                overlay_id,
                custom_overlay: None,
                ..
            } if !overlay_id.is_empty() && !all_overlay_ids.contains(overlay_id.as_str()) => {
                push_warning(&mut warnings, ShareWarning::MissingOverlay(overlay_id.clone()));
            }
            _ => {}
        });
    }

    let has_new_content = !rules.is_empty() || !new_overlays.is_empty() || !new_presets.is_empty();
    Ok(PreparedShareImport {
        rules,
        overlays: new_overlays,
        macro_presets: new_presets,
        skipped_duplicates,
        warnings,
        has_new_content,
    })
}

/// Rewrites macro-preset and overlay references through the import id maps.
fn remap_library_refs(
    rule: &mut Rule,
    overlay_map: &HashMap<String, String>,
    preset_map: &HashMap<String, String>,
) {
    fn remap_actions(
        actions: &mut [Action],
        overlay_map: &HashMap<String, String>,
        preset_map: &HashMap<String, String>,
    ) {
        for action in actions {
            match action {
                Action::Macro {
                    preset_id: Some(preset_id),
                    ..
                } => {
                    if let Some(mapped) = preset_map.get(preset_id) {
                        *preset_id = mapped.clone();
                    }
                }
                Action::Overlay {
                    overlay_id,
                    custom_overlay: None,
                    ..
                } => {
                    if let Some(mapped) = overlay_map.get(overlay_id) {
                        *overlay_id = mapped.clone();
                    }
                }
                Action::IfElse {
                    then_actions,
                    else_if_branches,
                    else_actions,
                    ..
                } => {
                    remap_actions(then_actions, overlay_map, preset_map);
                    for branch in else_if_branches {
                        remap_actions(&mut branch.actions, overlay_map, preset_map);
                    }
                    remap_actions(else_actions, overlay_map, preset_map);
                }
                _ => {}
            }
        }
    }
    remap_actions(&mut rule.actions, overlay_map, preset_map);
    remap_actions(&mut rule.on_exit_actions, overlay_map, preset_map);
}

fn fresh_id(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{prefix}_{nanos:x}_{:x}",
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Content identity of a rule, ignoring ids, enable state and runtime counters.
///
/// Library references are replaced with the content identity of the referenced overlay or
/// preset: import rewrites clashing library ids to fresh ones, so a re-import must compare
/// what the references point at, not the ids themselves.
fn rule_fingerprint(
    rule: &Rule,
    overlay_hashes: &HashMap<String, String>,
    preset_hashes: &HashMap<String, String>,
) -> String {
    let mut clone = rule.clone();
    clone.id.clear();
    clone.clear_quota_runtime();
    clone.is_enabled = false;
    // Icon paths are sender-local and never shared, so they must not affect identity.
    clone.custom_icon_path = None;
    // A clashing rule id makes import append this marker; it is not part of the rule content.
    if let Some(name) = clone.name.strip_suffix(" (Imported)") {
        clone.name = name.to_string();
    }
    canonicalize_library_refs(&mut clone, overlay_hashes, preset_hashes);
    serde_json::to_string(&clone).unwrap_or_default()
}

/// Replaces macro-preset and overlay ids with the content hash of the referenced library item.
fn canonicalize_library_refs(
    rule: &mut Rule,
    overlay_hashes: &HashMap<String, String>,
    preset_hashes: &HashMap<String, String>,
) {
    fn canonicalize_actions(
        actions: &mut [Action],
        overlay_hashes: &HashMap<String, String>,
        preset_hashes: &HashMap<String, String>,
    ) {
        for action in actions {
            match action {
                Action::Macro {
                    preset_id: Some(preset_id),
                    ..
                } => {
                    if let Some(hash) = preset_hashes.get(preset_id) {
                        *preset_id = hash.clone();
                    }
                }
                Action::Overlay {
                    overlay_id,
                    custom_overlay: None,
                    ..
                } => {
                    if let Some(hash) = overlay_hashes.get(overlay_id) {
                        *overlay_id = hash.clone();
                    }
                }
                Action::IfElse {
                    then_actions,
                    else_if_branches,
                    else_actions,
                    ..
                } => {
                    canonicalize_actions(then_actions, overlay_hashes, preset_hashes);
                    for branch in else_if_branches {
                        canonicalize_actions(&mut branch.actions, overlay_hashes, preset_hashes);
                    }
                    canonicalize_actions(else_actions, overlay_hashes, preset_hashes);
                }
                _ => {}
            }
        }
    }
    canonicalize_actions(&mut rule.actions, overlay_hashes, preset_hashes);
    canonicalize_actions(&mut rule.on_exit_actions, overlay_hashes, preset_hashes);
}

fn overlay_content_index(overlays: &[OverlayDefinition]) -> HashMap<String, String> {
    overlays
        .iter()
        .map(|overlay| (overlay.id.clone(), overlay_content_hash(overlay)))
        .collect()
}

fn preset_content_index(presets: &[MacroPreset]) -> HashMap<String, String> {
    presets
        .iter()
        .map(|preset| (preset.id.clone(), preset_content_hash(preset)))
        .collect()
}

/// Content identity of an overlay, ignoring its library id.
fn overlay_content_hash(overlay: &OverlayDefinition) -> String {
    let mut canonical = overlay.clone();
    canonical.id.clear();
    content_hash(&canonical)
}

/// Content identity of a macro preset, ignoring its library id.
fn preset_content_hash(preset: &MacroPreset) -> String {
    let mut canonical = preset.clone();
    canonical.id.clear();
    content_hash(&canonical)
}

fn content_hash<T: Serialize>(value: &T) -> String {
    match serde_json::to_vec(value) {
        Ok(bytes) => {
            use std::fmt::Write as _;
            let digest = Sha256::digest(&bytes);
            let mut out = String::with_capacity(digest.len() * 2);
            for byte in digest {
                let _ = write!(out, "{byte:02x}");
            }
            out
        }
        Err(_) => String::new(),
    }
}

/// Rewrites the rule ids a bundled overlay's interactive elements point at.
///
/// Import may rename a rule (`fresh_id`) or deduplicate it onto an existing local rule, so a
/// button that still carries the sender's id would do nothing or trigger an unrelated local
/// rule. Targets that remain unresolved are reported through a typed warning.
fn remap_overlay_rule_targets(
    overlay: &mut OverlayDefinition,
    rule_id_map: &HashMap<String, String>,
    known_rule_ids: &HashSet<String>,
    warnings: &mut Vec<ShareWarning>,
) {
    fn remap_target(
        rule_id: &mut String,
        rule_id_map: &HashMap<String, String>,
        known_rule_ids: &HashSet<String>,
        warnings: &mut Vec<ShareWarning>,
    ) {
        if rule_id.trim().is_empty() {
            return;
        }
        if let Some(mapped) = rule_id_map.get(rule_id) {
            *rule_id = mapped.clone();
        } else if !known_rule_ids.contains(rule_id.as_str()) {
            push_warning(warnings, ShareWarning::MissingRule(rule_id.clone()));
        }
    }

    fn remap_click_action(
        action: &mut OverlayClickAction,
        rule_id_map: &HashMap<String, String>,
        known_rule_ids: &HashSet<String>,
        warnings: &mut Vec<ShareWarning>,
    ) {
        let rule_id = match action {
            OverlayClickAction::TriggerRule { rule_id }
            | OverlayClickAction::ToggleRule { rule_id } => rule_id,
            _ => return,
        };
        remap_target(rule_id, rule_id_map, known_rule_ids, warnings);
    }

    fn remap_optional_action(
        action: &mut Option<OverlayClickAction>,
        rule_id_map: &HashMap<String, String>,
        known_rule_ids: &HashSet<String>,
        warnings: &mut Vec<ShareWarning>,
    ) {
        if let Some(action) = action {
            remap_click_action(action, rule_id_map, known_rule_ids, warnings);
        }
    }

    fn remap_optional_target(
        rule_id: &mut Option<String>,
        rule_id_map: &HashMap<String, String>,
        known_rule_ids: &HashSet<String>,
        warnings: &mut Vec<ShareWarning>,
    ) {
        if let Some(rule_id) = rule_id {
            remap_target(rule_id, rule_id_map, known_rule_ids, warnings);
        }
    }

    for element in &mut overlay.elements {
        match element {
            OverlayElement::Text { on_click, .. }
            | OverlayElement::Image { on_click, .. }
            | OverlayElement::Shape { on_click, .. }
            | OverlayElement::SystemTelemetry { on_click, .. } => {
                remap_optional_action(on_click, rule_id_map, known_rule_ids, warnings);
            }
            OverlayElement::ClockTimer { on_finish, .. } => {
                remap_optional_action(on_finish, rule_id_map, known_rule_ids, warnings);
            }
            OverlayElement::InteractiveControl { control, .. } => match control {
                InteractiveControlKind::Button { action, .. } => {
                    remap_optional_action(action, rule_id_map, known_rule_ids, warnings);
                }
                InteractiveControlKind::ToggleSwitch {
                    bind_rule_id,
                    on_enable,
                    on_disable,
                    ..
                } => {
                    remap_optional_target(bind_rule_id, rule_id_map, known_rule_ids, warnings);
                    remap_optional_action(on_enable, rule_id_map, known_rule_ids, warnings);
                    remap_optional_action(on_disable, rule_id_map, known_rule_ids, warnings);
                }
                InteractiveControlKind::Slider { .. } => {}
            },
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Clipboard helpers
// ---------------------------------------------------------------------------

/// Copy text to system clipboard via both egui context and arboard for maximum reliability
pub fn copy_text_to_clipboard(ctx: &egui::Context, text: &str) {
    ctx.copy_text(text.to_string());
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text);
    }
}

/// Read text from system clipboard
pub fn read_text_from_clipboard() -> Result<String, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("Clipboard error: {e}"))?;
    cb.get_text().map_err(|e| format!("Clipboard empty or non-text: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        Action, MacroEvent, MacroOperation, OverlayOperation, ProcessMatchMode, ScriptType,
        Trigger, WaitMode, WaitTimeoutPolicy,
    };
    fn sample_rule(id: &str) -> Rule {
        Rule {
            id: id.into(),
            name: "Test Launch Rule".into(),
            description: "Launches game and kills bloat".into(),
            trigger: Some(Trigger::Process {
                process_name: "game.exe".into(),
                match_mode: ProcessMatchMode::Exact,
                single_instance: true,
            }),
            actions: vec![Action::StartProcess {
                file_path: "C:\\tools\\companion.exe".into(),
                arguments: "-silent".into(),
                working_directory: None,
                run_as_admin: false,
                only_if_not_running: true,
                run_hidden: true,
                close_on_exit: true,
            }],
            on_exit_actions: vec![Action::Notification {
                title: "Game Exited".into(),
                message: "Cleaning up".into(),
            }],
            custom_icon_path: Some("C:\\games\\game.exe".into()),
            auto_process_icon: true,
            ..Default::default()
        }
    }

    fn preset(id: &str) -> MacroPreset {
        MacroPreset {
            id: id.into(),
            name: format!("Preset {id}"),
            events: vec![],
            repeat: 1,
            speed: 1.0,
            infinite: false,
        }
    }

    fn overlay(id: &str) -> OverlayDefinition {
        let mut overlay = OverlayDefinition::default_new_overlay();
        overlay.id = id.into();
        overlay.name = format!("Overlay {id}");
        overlay
    }

    /// Bundled overlay with sender-local image and button icon paths.
    fn asset_overlay(id: &str) -> OverlayDefinition {
        let mut overlay = overlay(id);
        overlay.elements = vec![
            OverlayElement::Image {
                id: "img_1".into(),
                path: "C:\\Users\\Sender\\Pictures\\hud.png".into(),
                x: 0.0,
                y: 0.0,
                width: 64.0,
                height: 64.0,
                opacity: 1.0,
                lock_aspect: true,
                rotation: 0.0,
                on_click: None,
            },
            OverlayElement::InteractiveControl {
                id: "btn_1".into(),
                control: InteractiveControlKind::Button {
                    label: "Start".into(),
                    icon: Some("C:\\Users\\Sender\\Pictures\\start.ico".into()),
                    action: None,
                },
                color: [255, 255, 255, 255],
                bg_color: [0, 0, 0, 0],
                active_color: [70, 160, 255, 255],
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 32.0,
                rotation: 0.0,
            },
        ];
        overlay
    }

    /// Bundled overlay whose button and toggle point at `rule_id`, plus a button that points
    /// at a rule missing from both the code and the local rule set.
    fn click_overlay(id: &str, rule_id: &str) -> OverlayDefinition {
        let control = |id: &str, kind| OverlayElement::InteractiveControl {
            id: id.into(),
            control: kind,
            color: [255, 255, 255, 255],
            bg_color: [0, 0, 0, 0],
            active_color: [70, 160, 255, 255],
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 32.0,
            rotation: 0.0,
        };
        let mut overlay = overlay(id);
        overlay.elements = vec![
            control(
                "btn",
                InteractiveControlKind::Button {
                    label: "Run".into(),
                    icon: None,
                    action: Some(OverlayClickAction::TriggerRule {
                        rule_id: rule_id.into(),
                    }),
                },
            ),
            control(
                "toggle",
                InteractiveControlKind::ToggleSwitch {
                    label: "Toggle".into(),
                    is_active: false,
                    bind_rule_id: Some(rule_id.into()),
                    on_enable: Some(OverlayClickAction::ToggleRule {
                        rule_id: rule_id.into(),
                    }),
                    on_disable: Some(OverlayClickAction::TriggerRule {
                        rule_id: rule_id.into(),
                    }),
                },
            ),
            OverlayElement::Text {
                id: "ghost".into(),
                text: "Ghost".into(),
                font_size: 12.0,
                color: [255, 255, 255, 255],
                x: 0.0,
                y: 0.0,
                width: None,
                height: None,
                lock_aspect: false,
                bold: false,
                font_family: "Segoe UI".into(),
                rotation: 0.0,
                on_click: Some(OverlayClickAction::TriggerRule {
                    rule_id: "ghost_rule".into(),
                }),
            },
        ];
        overlay
    }

    fn encode(payload: &SharePayload) -> String {
        compress_and_encode(payload).expect("encode")
    }

    #[test]
    fn test_single_rule_roundtrip() {
        let rule = sample_rule("test_rule_1");
        let (payload, warnings) = build_payload(vec![rule.clone()], &[], &[]);
        assert_eq!(warnings, vec![ShareWarning::CustomIconDropped]);

        let encoded = encode(&payload);
        assert!(encoded.starts_with("SYN1:"));
        assert_eq!(encoded, encoded.trim());

        let decoded = decode_and_decompress(&encoded).expect("decode");
        assert_eq!(decoded.rules.len(), 1);
        assert_eq!(decoded.rules[0].id, rule.id);
        assert_eq!(decoded.rules[0].name, rule.name);
        assert!(decoded.rules[0].custom_icon_path.is_none());
        assert_eq!(decoded.app_version, env!("CARGO_PKG_VERSION"));
        assert!(!decoded.created_at.is_empty());
        assert_eq!(decoded, payload, "decoding must restore the payload exactly");
    }

    #[test]
    fn test_share_code_strips_default_fields_losslessly() {
        let mut rule = sample_rule("defaults_rule");
        rule.custom_icon_path = None;
        rule.trigger = Some(Trigger::Process {
            process_name: "game.exe".into(),
            match_mode: ProcessMatchMode::Exact,
            single_instance: false,
        });
        rule.actions = vec![
            Action::StartProcess {
                file_path: "app.exe".into(),
                arguments: String::new(),
                working_directory: None,
                run_as_admin: false,
                only_if_not_running: false,
                run_hidden: false,
                close_on_exit: false,
            },
            Action::Macro {
                preset_id: None,
                operation: MacroOperation::Run,
                events: vec![],
                repeat: 1,
                speed: 1.0,
                infinite: false,
            },
            Action::Wait {
                mode: WaitMode::Timer,
                seconds: 1.0,
                condition: None,
                timeout_sec: 30.0,
                timeout_policy: WaitTimeoutPolicy::Abort,
            },
            Action::Overlay {
                overlay_id: "library_overlay".into(),
                custom_overlay: None,
                operation: OverlayOperation::Show,
                override_text: None,
                duration_ms: None,
                close_on_exit: false,
            },
        ];
        let mut second = rule.clone();
        second.id = "defaults_rule_2".into();
        let payload = SharePayload::rules_only(vec![rule, second]);

        let json = String::from_utf8(payload_json(&payload).expect("json")).unwrap();
        for stripped in [
            "\"run_as_admin\"",
            "\"only_if_not_running\"",
            "\"run_hidden\"",
            "\"repeat\"",
            "\"speed\"",
            "\"infinite\"",
            "\"timeout_sec\"",
            "\"timeout_policy\"",
            "\"operation\":\"run\"",
            "\"match_mode\"",
            "\"single_instance\"",
            "\"close_on_exit\"",
        ] {
            assert!(!json.contains(stripped), "{stripped} must be stripped: {json}");
        }

        let decoded = decode_and_decompress(&encode(&payload)).expect("decode");
        assert_eq!(decoded, payload, "stripped defaults must be restored by serde");
    }

    #[test]
    fn test_batch_rules_roundtrip() {
        let rules: Vec<Rule> = (0..5)
            .map(|i| Rule {
                id: format!("rule_{i}"),
                name: format!("Automation Rule {i}"),
                description: format!("Description for {i}"),
                actions: vec![Action::Notification {
                    title: format!("Title {i}"),
                    message: format!("Message {i}"),
                }],
                ..Default::default()
            })
            .collect();
        let (payload, warnings) = build_payload(rules.clone(), &[], &[]);
        assert!(warnings.is_empty());

        let decoded = decode_and_decompress(&encode(&payload)).expect("batch decode");
        assert_eq!(decoded.rules.len(), 5);
        assert_eq!(decoded.rules[0].name, "Automation Rule 0");
        assert_eq!(decoded.rules[4].name, "Automation Rule 4");
    }

    #[test]
    fn test_build_payload_bundles_referenced_library_items() {
        let library_presets = vec![preset("preset_alpha"), preset("preset_unused")];
        let library_overlays = vec![overlay("overlay_alpha"), overlay("overlay_unused")];

        let mut rule = sample_rule("deps_rule");
        rule.custom_icon_path = None;
        rule.actions = vec![
            Action::Macro {
                preset_id: Some("preset_alpha".into()),
                operation: crate::models::MacroOperation::Run,
                events: vec![MacroEvent::KeyDown { key: "A".into() }],
                repeat: 1,
                speed: 1.0,
                infinite: false,
            },
            Action::Overlay {
                overlay_id: "overlay_alpha".into(),
                custom_overlay: None,
                operation: crate::models::OverlayOperation::Show,
                override_text: None,
                duration_ms: None,
                close_on_exit: false,
            },
        ];

        let (payload, warnings) = build_payload(vec![rule], &library_overlays, &library_presets);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(payload.macro_presets.len(), 1);
        assert_eq!(payload.macro_presets[0].id, "preset_alpha");
        assert_eq!(payload.overlays.len(), 1);
        assert_eq!(payload.overlays[0].id, "overlay_alpha");
    }

    #[test]
    fn test_build_payload_warns_about_missing_dependencies_and_sensitive_actions() {
        let mut rule = sample_rule("risky_rule");
        rule.custom_icon_path = None;
        rule.actions = vec![
            Action::Macro {
                preset_id: Some("missing_preset".into()),
                operation: crate::models::MacroOperation::Run,
                events: vec![],
                repeat: 1,
                speed: 1.0,
                infinite: false,
            },
            Action::Overlay {
                overlay_id: "missing_overlay".into(),
                custom_overlay: None,
                operation: crate::models::OverlayOperation::Show,
                override_text: None,
                duration_ms: None,
                close_on_exit: false,
            },
            Action::ExecuteScript {
                interpreter: ScriptType::PowerShell,
                script: "echo hi".into(),
                run_in_window: false,
            },
            Action::StartProcess {
                file_path: "cmd.exe".into(),
                arguments: String::new(),
                working_directory: None,
                run_as_admin: true,
                only_if_not_running: false,
                run_hidden: true,
                close_on_exit: false,
            },
            Action::HttpWebhook {
                url: "https://example.com/hook".into(),
                method: "POST".into(),
                headers: None,
                payload: None,
            },
        ];

        let (_, warnings) = build_payload(vec![rule], &[], &[]);
        for expected in [
            ShareWarning::MissingMacroPreset("missing_preset".into()),
            ShareWarning::MissingOverlay("missing_overlay".into()),
            ShareWarning::SensitiveScript,
            ShareWarning::SensitiveAdminProcess,
            ShareWarning::SensitiveWebhook,
        ] {
            assert!(warnings.contains(&expected), "missing {expected:?} in {warnings:?}");
        }
    }

    #[test]
    fn test_share_code_survives_corruption_checks() {
        let (payload, _) = build_payload(vec![sample_rule("checksum_rule")], &[], &[]);
        let encoded = encode(&payload);
        let body = encoded.strip_prefix(SHARE_PREFIX).unwrap();
        let mut bytes = BASE64_URL_SAFE_NO_PAD.decode(body).unwrap();
        // Flip a bit in the middle of the compressed body (after the checksum prefix).
        let middle = CHECKSUM_LEN + (bytes.len() - CHECKSUM_LEN) / 2;
        bytes[middle] ^= 0x40;
        let corrupted = format!(
            "{SHARE_PREFIX}{}",
            BASE64_URL_SAFE_NO_PAD.encode(bytes)
        );
        let error = decode_and_decompress(&corrupted).expect_err("corruption must be caught");
        assert!(
            matches!(
                error,
                ShareError::ChecksumMismatch
                    | ShareError::CorruptedPayload
                    | ShareError::InvalidPayload
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn test_legacy_ph1_code_is_rejected() {
        let (payload, _) = build_payload(vec![sample_rule("legacy_rule")], &[], &[]);
        let json = serde_json::to_vec(&payload).unwrap();
        let legacy = format!("PH1:{}", BASE64_STANDARD.encode(&json));
        assert_eq!(
            decode_and_decompress(&legacy).unwrap_err(),
            ShareError::LegacyFormat
        );
    }

    #[test]
    fn test_share_code_tolerates_wrapping_and_line_breaks() {
        let (payload, _) = build_payload(vec![sample_rule("wrapped_rule")], &[], &[]);
        let encoded = encode(&payload);

        let decoded = decode_and_decompress(&format!("```\n{encoded}\n```\n")).expect("fenced");
        assert_eq!(decoded.rules[0].id, "wrapped_rule");

        // Split the body across lines the way a chat client might wrap it.
        let body = encoded.strip_prefix(SHARE_PREFIX).unwrap();
        let broken: String = body
            .as_bytes()
            .chunks(37)
            .map(|chunk| std::str::from_utf8(chunk).unwrap().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let decoded = decode_and_decompress(&format!("{SHARE_PREFIX}\n{broken}")).expect("wrapped");
        assert_eq!(decoded.rules[0].id, "wrapped_rule");
    }

    #[test]
    fn test_whitespace_and_quote_tolerance() {
        let (payload, _) = build_payload(vec![sample_rule("quoted_rule")], &[], &[]);
        let encoded = encode(&payload);
        let decoded = decode_and_decompress(&format!(" \"{encoded}\" ")).expect("quoted");
        assert_eq!(decoded.rules[0].id, "quoted_rule");
    }

    #[test]
    fn test_raw_json_paste_still_imports() {
        let rule = sample_rule("json_rule");
        let json = serde_json::to_string(&rule).unwrap();
        let decoded = decode_and_decompress(&json).expect("raw rule json");
        assert_eq!(decoded.rules.len(), 1);
        assert_eq!(decoded.rules[0].id, "json_rule");

        let batch_json = serde_json::to_string(&vec![sample_rule("a"), sample_rule("b")]).unwrap();
        let decoded = decode_and_decompress(&batch_json).expect("raw batch json");
        assert_eq!(decoded.rules.len(), 2);
    }

    #[test]
    fn test_import_rejects_unknown_keys_and_invalid_values() {
        let typo = r#"{"id":"r","name":"R","max_execution":5,"actions":[]}"#;
        assert_eq!(decode_and_decompress(typo), Err(ShareError::InvalidPayload));

        let nested = r#"{"id":"r","name":"R","actions":[{"$action_type":"notification","title":"T","message":"M","mesage":"typo"}]}"#;
        assert_eq!(decode_and_decompress(nested), Err(ShareError::InvalidPayload));

        // A tag colour that is not ASCII hex must not reach the renderer.
        let bad_tag = r#"{"id":"r","name":"R","tags":[{"name":"T","color":"aé€"}],"actions":[]}"#;
        assert_eq!(decode_and_decompress(bad_tag), Err(ShareError::InvalidPayload));

        // A Daily schedule without its time used to import and then never fire.
        let bad_schedule = r#"{"id":"r","name":"R","trigger":{"$trigger_type":"schedule","schedule_type":"daily","interval_sec":0},"actions":[]}"#;
        assert_eq!(
            decode_and_decompress(bad_schedule),
            Err(ShareError::InvalidPayload)
        );
    }

    #[test]
    fn test_prepare_import_normalizes_overlay_ids() {
        let mut rule = sample_rule("overlay_rule");
        rule.actions = vec![Action::Overlay {
            overlay_id: "outside".into(),
            custom_overlay: Some(Box::new(crate::models::OverlayDefinition {
                id: "inside".into(),
                name: "HUD".into(),
                ..Default::default()
            })),
            operation: OverlayOperation::Show,
            override_text: None,
            duration_ms: None,
            close_on_exit: false,
        }];

        let prepared = prepare_import(SharePayload::rules_only(vec![rule]), &[], &[], &[], true)
            .expect("prepare");
        match &prepared.rules[0].actions[0] {
            Action::Overlay {
                overlay_id,
                custom_overlay,
                ..
            } => {
                assert_eq!(overlay_id, "outside");
                assert_eq!(
                    custom_overlay.as_ref().map(|definition| definition.id.as_str()),
                    Some("outside")
                );
            }
            other => panic!("expected an overlay action, got {other:?}"),
        }
    }

    #[test]
    fn test_checksum_bounds_the_decompressed_payload() {
        let oversized = vec![b' '; MAX_PAYLOAD_BYTES + 4096];
        let checksum = Sha256::digest(&oversized);
        let mut compressed = Vec::new();
        let params = BrotliEncoderParams {
            quality: 1,
            ..Default::default()
        };
        brotli::BrotliCompress(&mut oversized.as_slice(), &mut compressed, &params).unwrap();
        let mut framed = Vec::new();
        framed.extend_from_slice(&checksum[..CHECKSUM_LEN]);
        framed.extend_from_slice(&compressed);
        let code = format!("{SHARE_PREFIX}{}", BASE64_URL_SAFE_NO_PAD.encode(framed));

        assert_eq!(
            decode_and_decompress(&code).unwrap_err(),
            ShareError::PayloadTooLarge
        );
    }

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
    fn test_decoder_never_panics_on_garbage() {
        let corpus = [
            "",
            "SYN1:",
            "SYN1:AAAA",
            "SYN1:!!!!",
            "PH1:abcd",
            "not a code",
            "{\"rules\":[]}",
            "[]",
            "SYN1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ];
        for input in corpus {
            let _ = decode_and_decompress(input);
        }
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_+/=";
        let mut seed: u64 = 0x9E3779B97F4A7C15;
        for _ in 0..200 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let len = (seed % 64) as usize;
            let mut body = String::new();
            for _ in 0..len {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                body.push(alphabet[(seed % alphabet.len() as u64) as usize] as char);
            }
            let _ = decode_and_decompress(&format!("{SHARE_PREFIX}{body}"));
        }
    }

    #[test]
    fn test_dictionary_shrinks_share_codes() {
        let rules: Vec<Rule> = (0..5)
            .map(|i| {
                let mut rule = sample_rule(&format!("rule_{i}"));
                rule.name = format!("Workflow {i}");
                rule.description =
                    "Launches the game, trims memory and shows a notification".into();
                rule
            })
            .collect();
        let (payload, _) = build_payload(rules, &[], &[]);
        let json = serde_json::to_vec(&payload).unwrap();

        let dictionary_code = compress_and_encode(&payload).expect("encode");

        // Same framing and quality, but without the format dictionary.
        let params = BrotliEncoderParams {
            quality: SHARE_QUALITY,
            ..Default::default()
        };
        let mut plain_compressed = Vec::new();
        brotli::BrotliCompress(&mut json.as_slice(), &mut plain_compressed, &params)
            .expect("plain brotli");
        let digest = Sha256::digest(&json);
        let mut framed = Vec::new();
        framed.extend_from_slice(&digest[..CHECKSUM_LEN]);
        framed.extend_from_slice(&plain_compressed);
        let plain_code = format!("{SHARE_PREFIX}{}", BASE64_URL_SAFE_NO_PAD.encode(framed));

        println!(
            "share code with dictionary: {} bytes, without: {} bytes",
            dictionary_code.len(),
            plain_code.len()
        );
        assert!(
            dictionary_code.len() * 100 < plain_code.len() * 95,
            "dictionary must shrink the code by at least 5% ({} vs {})",
            dictionary_code.len(),
            plain_code.len()
        );
    }

    #[test]
    fn test_prepare_import_resets_runtime_and_disables_rules() {
        let mut shared = sample_rule("shared_rule");
        shared.execution_count = 42;
        shared.last_executed_at = Some("2020-01-01T00:00:00Z".into());
        shared.quota_window_start = Some("2020-01-01 00:00:00".into());
        shared.is_enabled = true;
        let (payload, _) = build_payload(vec![shared], &[], &[]);

        let prepared = prepare_import(payload, &[], &[], &[], false).expect("prepare");
        assert_eq!(prepared.rules.len(), 1);
        assert!(!prepared.rules[0].is_enabled);
        assert_eq!(prepared.rules[0].execution_count, 0);
        assert!(prepared.rules[0].last_executed_at.is_none());
        assert!(prepared.rules[0].quota_window_start.is_none());
        assert!(prepared.has_new_content);

        let enabled = prepare_import(
            SharePayload::rules_only(vec![sample_rule("enable_rule")]),
            &[],
            &[],
            &[],
            true,
        )
        .expect("prepare enabled");
        assert!(enabled.rules[0].is_enabled);
    }

    #[test]
    fn test_prepare_import_skips_duplicates() {
        let duplicate = sample_rule("dup_rule");
        let (payload, _) = build_payload(vec![duplicate.clone()], &[], &[]);
        let prepared =
            prepare_import(payload, std::slice::from_ref(&duplicate), &[], &[], false).expect("prepare");
        assert!(prepared.rules.is_empty());
        assert_eq!(prepared.skipped_duplicates, 1);
        assert!(!prepared.has_new_content);
    }

    #[test]
    fn test_prepare_import_renames_clashing_rule_ids() {
        let existing = Rule {
            id: "same_id".into(),
            name: "Existing".into(),
            ..Default::default()
        };
        let incoming = Rule {
            id: "same_id".into(),
            name: "Incoming".into(),
            ..Default::default()
        };
        let prepared = prepare_import(
            SharePayload::rules_only(vec![incoming]),
            std::slice::from_ref(&existing),
            &[],
            &[],
            false,
        )
        .expect("prepare");
        assert_eq!(prepared.rules.len(), 1);
        assert_ne!(prepared.rules[0].id, "same_id");
        assert!(prepared.rules[0].name.ends_with("(Imported)"));
    }

    #[test]
    fn test_prepare_import_rewrites_clashing_library_ids() {
        let local_overlay = overlay("overlay_shared");
        let mut local_variant = local_overlay.clone();
        local_variant.name = "Local variant".into();

        let mut rule = sample_rule("dep_rule");
        rule.custom_icon_path = None;
        rule.actions = vec![Action::Overlay {
            overlay_id: "overlay_shared".into(),
            custom_overlay: None,
            operation: crate::models::OverlayOperation::Show,
            override_text: None,
            duration_ms: None,
            close_on_exit: false,
        }];
        let (payload, _) = build_payload(
            vec![rule],
            std::slice::from_ref(&local_overlay),
            &[],
        );
        assert_eq!(payload.overlays.len(), 1);

        let prepared = prepare_import(
            payload,
            &[],
            std::slice::from_ref(&local_variant),
            &[],
            false,
        )
        .expect("prepare");
        assert_eq!(prepared.overlays.len(), 1);
        let new_overlay_id = prepared.overlays[0].id.clone();
        assert_ne!(new_overlay_id, "overlay_shared");
        match &prepared.rules[0].actions[0] {
            Action::Overlay { overlay_id, .. } => assert_eq!(overlay_id, &new_overlay_id),
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn test_prepare_import_keeps_identical_library_items() {
        let local_overlay = overlay("overlay_same");
        let mut rule = sample_rule("same_overlay_rule");
        rule.custom_icon_path = None;
        rule.actions = vec![Action::Overlay {
            overlay_id: "overlay_same".into(),
            custom_overlay: None,
            operation: crate::models::OverlayOperation::Show,
            override_text: None,
            duration_ms: None,
            close_on_exit: false,
        }];
        let (payload, _) = build_payload(
            vec![rule],
            std::slice::from_ref(&local_overlay),
            &[],
        );
        let prepared = prepare_import(
            payload,
            &[],
            std::slice::from_ref(&local_overlay),
            &[],
            false,
        )
        .expect("prepare");
        assert!(prepared.overlays.is_empty(), "identical overlay must not be re-added");
        match &prepared.rules[0].actions[0] {
            Action::Overlay { overlay_id, .. } => assert_eq!(overlay_id, "overlay_same"),
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn test_prepare_import_bundles_macro_presets() {
        let library_preset = preset("preset_shared");
        let mut rule = sample_rule("macro_rule");
        rule.custom_icon_path = None;
        rule.actions = vec![Action::Macro {
            preset_id: Some("preset_shared".into()),
            operation: crate::models::MacroOperation::Run,
            events: vec![],
            repeat: 1,
            speed: 1.0,
            infinite: false,
        }];
        let (payload, _) = build_payload(vec![rule], &[], std::slice::from_ref(&library_preset));
        assert_eq!(payload.macro_presets.len(), 1);

        let prepared = prepare_import(payload, &[], &[], &[], false).expect("prepare");
        assert_eq!(prepared.macro_presets.len(), 1);
        assert!(prepared.warnings.is_empty());
        match &prepared.rules[0].actions[0] {
            Action::Macro {
                preset_id: Some(preset_id),
                ..
            } => assert_eq!(preset_id, "preset_shared"),
            other => panic!("unexpected action: {other:?}"),
        }
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
    fn test_rule_json_omits_default_fields() {
        let rule = Rule {
            id: "compact".into(),
            name: "Compact".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&rule).unwrap();
        for omitted in [
            "execution_count",
            "is_enabled",
            "cooldown_seconds",
            "stop_on_error",
            "auto_process_icon",
            "custom_icon_path",
            "last_executed_at",
            "quota_period",
            "quota_window_start",
        ] {
            assert!(!json.contains(omitted), "{omitted} must be omitted: {json}");
        }
        let decoded: Rule = serde_json::from_str(&json).unwrap();
        assert!(decoded.is_enabled);
        assert!(decoded.stop_on_error);
        assert_eq!(decoded.execution_count, 0);
        assert_eq!(decoded.cooldown_seconds, 0.0);
        assert!(!decoded.auto_process_icon);
    }

    #[test]
    fn test_prepare_import_strips_foreign_icon_paths() {
        let rule = sample_rule("icon_rule");
        assert!(rule.custom_icon_path.is_some());
        let prepared = prepare_import(
            SharePayload::rules_only(vec![rule]),
            &[],
            &[],
            &[],
            false,
        )
        .expect("prepare");
        assert!(prepared.rules[0].custom_icon_path.is_none());
        assert!(prepared.warnings.contains(&ShareWarning::CustomIconDropped));
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
    fn test_event_condition_idle_zero_roundtrip() {
        use crate::models::{RuleCondition, RuleConditionItem, SystemEventType};

        let mut rule = sample_rule("idle_zero_rule");
        rule.trigger = None;
        rule.conditions = vec![RuleConditionItem::new(RuleCondition::Event {
            event_type: SystemEventType::UserIdle,
            idle_duration_sec: 0,
        })];

        let (payload, _) = build_payload(vec![rule], &[], &[]);
        let json = String::from_utf8(payload_json(&payload).expect("json")).unwrap();
        assert!(
            json.contains("\"idle_duration_sec\":0"),
            "an intentional zero must stay in the payload: {json}"
        );

        let decoded = decode_and_decompress(&encode(&payload)).expect("decode");
        assert_eq!(decoded, payload, "idle_duration_sec 0 must round-trip");
        match &decoded.rules[0].conditions[0].condition {
            RuleCondition::Event {
                idle_duration_sec, ..
            } => assert_eq!(*idle_duration_sec, 0),
            other => panic!("expected an event condition, got {other:?}"),
        }
    }

    #[test]
    fn test_prepare_import_deduplicates_same_code_twice() {
        // The local library already holds the code's overlay id with different content, so the
        // first import renames the bundled overlay to a fresh id.
        let mut local_variant = overlay("overlay_shared");
        local_variant.name = "Local variant".into();
        let mut shared_overlay = overlay("overlay_shared");
        shared_overlay.name = "Shared variant".into();

        let mut rule = sample_rule("deps_rule");
        rule.custom_icon_path = None;
        rule.actions = vec![Action::Overlay {
            overlay_id: "overlay_shared".into(),
            custom_overlay: None,
            operation: OverlayOperation::Show,
            override_text: None,
            duration_ms: None,
            close_on_exit: false,
        }];
        let (payload, _) =
            build_payload(vec![rule], std::slice::from_ref(&shared_overlay), &[]);
        assert_eq!(payload.overlays.len(), 1);

        let first = prepare_import(
            payload.clone(),
            &[],
            std::slice::from_ref(&local_variant),
            &[],
            false,
        )
        .expect("first import");
        assert_eq!(first.rules.len(), 1);
        assert_eq!(first.overlays.len(), 1);
        assert_ne!(first.overlays[0].id, "overlay_shared");

        let mut local_overlays = vec![local_variant];
        local_overlays.extend(first.overlays.iter().cloned());
        let second =
            prepare_import(payload, &first.rules, &local_overlays, &[], false).expect("second import");
        assert!(
            second.rules.is_empty(),
            "the same rule must not be imported twice"
        );
        assert_eq!(second.skipped_duplicates, 1);
        assert!(
            second.overlays.is_empty(),
            "the same overlay must not be appended twice"
        );
        assert!(!second.has_new_content);
    }

    #[test]
    fn test_prepare_import_deduplicates_renamed_rule_ids() {
        let existing = Rule {
            id: "same_id".into(),
            name: "Existing".into(),
            ..Default::default()
        };
        let incoming = Rule {
            id: "same_id".into(),
            name: "Incoming".into(),
            ..Default::default()
        };
        let payload = SharePayload::rules_only(vec![incoming]);
        let first = prepare_import(
            payload.clone(),
            std::slice::from_ref(&existing),
            &[],
            &[],
            false,
        )
        .expect("first import");
        assert_eq!(first.rules.len(), 1);
        assert!(first.rules[0].name.ends_with("(Imported)"));

        let second = prepare_import(
            payload,
            &[existing, first.rules[0].clone()],
            &[],
            &[],
            false,
        )
        .expect("second import");
        assert!(
            second.rules.is_empty(),
            "the import marker must not defeat deduplication"
        );
        assert_eq!(second.skipped_duplicates, 1);
    }

    #[test]
    fn test_prepare_import_remaps_overlay_click_targets() {
        let existing = Rule {
            id: "shared_target".into(),
            name: "Local rule".into(),
            ..Default::default()
        };
        let incoming = Rule {
            id: "shared_target".into(),
            name: "Shared rule".into(),
            ..Default::default()
        };
        let mut payload = SharePayload::rules_only(vec![incoming]);
        payload.overlays = vec![click_overlay("overlay_buttons", "shared_target")];

        let prepared = prepare_import(
            payload,
            std::slice::from_ref(&existing),
            &[],
            &[],
            false,
        )
        .expect("prepare");
        assert_eq!(prepared.rules.len(), 1);
        let renamed = prepared.rules[0].id.clone();
        assert_ne!(renamed, "shared_target");

        let overlay = &prepared.overlays[0];
        match &overlay.elements[0] {
            OverlayElement::InteractiveControl {
                control:
                    InteractiveControlKind::Button {
                        action: Some(OverlayClickAction::TriggerRule { rule_id }),
                        ..
                    },
                ..
            } => assert_eq!(rule_id, &renamed),
            other => panic!("unexpected element: {other:?}"),
        }
        match &overlay.elements[1] {
            OverlayElement::InteractiveControl {
                control:
                    InteractiveControlKind::ToggleSwitch {
                        bind_rule_id,
                        on_enable,
                        on_disable,
                        ..
                    },
                ..
            } => {
                assert_eq!(bind_rule_id.as_deref(), Some(renamed.as_str()));
                assert!(matches!(
                    on_enable,
                    Some(OverlayClickAction::ToggleRule { rule_id }) if rule_id == &renamed
                ));
                assert!(matches!(
                    on_disable,
                    Some(OverlayClickAction::TriggerRule { rule_id }) if rule_id == &renamed
                ));
            }
            other => panic!("unexpected element: {other:?}"),
        }
        assert!(
            prepared
                .warnings
                .contains(&ShareWarning::MissingRule("ghost_rule".into())),
            "unresolved click targets must warn: {:?}",
            prepared.warnings
        );
    }

    #[test]
    fn test_build_payload_strips_overlay_asset_paths() {
        let library_overlay = asset_overlay("overlay_assets");
        let mut rule = sample_rule("asset_rule");
        rule.custom_icon_path = None;
        rule.actions = vec![Action::Overlay {
            overlay_id: "overlay_assets".into(),
            custom_overlay: None,
            operation: OverlayOperation::Show,
            override_text: None,
            duration_ms: None,
            close_on_exit: false,
        }];
        let (payload, warnings) = build_payload(
            vec![rule],
            std::slice::from_ref(&library_overlay),
            &[],
        );
        assert!(
            warnings.contains(&ShareWarning::OverlayAssetDropped),
            "{warnings:?}"
        );
        match &payload.overlays[0].elements[0] {
            OverlayElement::Image { path, .. } => {
                assert!(path.is_empty(), "absolute image path must be dropped")
            }
            other => panic!("unexpected element: {other:?}"),
        }
        match &payload.overlays[0].elements[1] {
            OverlayElement::InteractiveControl {
                control: InteractiveControlKind::Button { icon, .. },
                ..
            } => assert!(icon.is_none(), "absolute button icon must be dropped"),
            other => panic!("unexpected element: {other:?}"),
        }
        // The sender's library entry must stay untouched.
        match &library_overlay.elements[0] {
            OverlayElement::Image { path, .. } => assert!(!path.is_empty()),
            other => panic!("unexpected element: {other:?}"),
        }

        // An overlay embedded directly in the rule is stripped too.
        let mut embedded_rule = sample_rule("embedded_asset_rule");
        embedded_rule.custom_icon_path = None;
        embedded_rule.actions = vec![Action::Overlay {
            overlay_id: "embedded_overlay".into(),
            custom_overlay: Some(Box::new(asset_overlay("embedded_overlay"))),
            operation: OverlayOperation::Show,
            override_text: None,
            duration_ms: None,
            close_on_exit: false,
        }];
        let (payload, warnings) = build_payload(vec![embedded_rule], &[], &[]);
        assert!(warnings.contains(&ShareWarning::OverlayAssetDropped));
        match &payload.rules[0].actions[0] {
            Action::Overlay {
                custom_overlay: Some(definition),
                ..
            } => match &definition.elements[0] {
                OverlayElement::Image { path, .. } => assert!(path.is_empty()),
                other => panic!("unexpected element: {other:?}"),
            },
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn test_prepare_import_strips_overlay_asset_paths() {
        let mut rule = sample_rule("incoming_asset_rule");
        rule.custom_icon_path = None;
        rule.actions = vec![Action::Overlay {
            overlay_id: "incoming_assets".into(),
            custom_overlay: None,
            operation: OverlayOperation::Show,
            override_text: None,
            duration_ms: None,
            close_on_exit: false,
        }];
        let mut payload = SharePayload::rules_only(vec![rule]);
        payload.overlays = vec![asset_overlay("incoming_assets")];

        let prepared = prepare_import(payload, &[], &[], &[], false).expect("prepare");
        assert!(
            prepared.warnings.contains(&ShareWarning::OverlayAssetDropped),
            "{:?}",
            prepared.warnings
        );
        match &prepared.overlays[0].elements[0] {
            OverlayElement::Image { path, .. } => assert!(path.is_empty()),
            other => panic!("unexpected element: {other:?}"),
        }
        match &prepared.overlays[0].elements[1] {
            OverlayElement::InteractiveControl {
                control: InteractiveControlKind::Button { icon, .. },
                ..
            } => assert!(icon.is_none()),
            other => panic!("unexpected element: {other:?}"),
        }
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

    #[test]
    fn test_checksum_rejects_one_bit_difference() {
        let (payload, _) = build_payload(vec![sample_rule("checksum_bit_rule")], &[], &[]);
        let encoded = encode(&payload);
        let body = encoded.strip_prefix(SHARE_PREFIX).unwrap();
        let framed = BASE64_URL_SAFE_NO_PAD.decode(body).unwrap();
        assert!(framed.len() > CHECKSUM_LEN);

        for bit in 0..CHECKSUM_LEN * 8 {
            let mut altered = framed.clone();
            altered[bit / 8] ^= 1 << (bit % 8);
            let code = format!("{SHARE_PREFIX}{}", BASE64_URL_SAFE_NO_PAD.encode(&altered));
            assert_eq!(
                decode_and_decompress(&code),
                Err(ShareError::ChecksumMismatch),
                "flipping checksum bit {bit} must be detected"
            );
        }
    }
}
