//! Share code encoding: payload serialization, Brotli compression and payload assembly.

use std::collections::HashSet;

use base64::prelude::*;
use brotli::enc::BrotliEncoderParams;
use sha2::{Digest, Sha256};

use crate::models::{Action, MacroPreset, OverlayDefinition, Rule};

use super::assets::{collect_sensitive_warnings, strip_overlay_asset_paths, strip_rule_overlay_assets};
use super::decode::validate_payload_limits;
use super::format::{
    ShareError, ShareWarning, CHECKSUM_LEN, SHARE_DICTIONARY, SHARE_PREFIX, SHARE_QUALITY,
    SHARE_VERSION, MAX_PAYLOAD_BYTES,
};
use super::payload::SharePayload;

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
        "event" => is_text("started"),
        "lifetime" => is_text("detach"),
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

pub(super) fn push_warning(warnings: &mut Vec<ShareWarning>, warning: ShareWarning) {
    if !warnings.contains(&warning) {
        warnings.push(warning);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::models::{
        MacroEvent, MacroOperation, OverlayOperation, ProcessMatchMode, ScriptType, Trigger,
        WaitMode, WaitTimeoutPolicy,
    };
    use crate::sharing::decode_and_decompress;
    use crate::sharing::payload::fixtures::{encode, overlay, preset, sample_rule};

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
            event: Default::default(),
        });
        rule.actions = vec![
            Action::StartProcess {
                file_path: "app.exe".into(),
                arguments: String::new(),
                working_directory: None,
                run_as_admin: false,
                only_if_not_running: false,
                run_hidden: false,
                lifetime: crate::models::ResourceLifetime::Detach,
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
                lifetime: crate::models::ResourceLifetime::Detach,
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
            "\"lifetime\"",
            "\"event\"",
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
                lifetime: crate::models::ResourceLifetime::Detach,
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
                lifetime: crate::models::ResourceLifetime::Detach,
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
                lifetime: crate::models::ResourceLifetime::Detach,
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
}
