//! Share import preparation: validation, id remapping, content deduplication and asset stripping.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::models::{
    Action, InteractiveControlKind, MacroPreset, OverlayClickAction, OverlayDefinition,
    OverlayElement, Rule,
};

use super::assets::{collect_sensitive_warnings, strip_overlay_asset_paths, strip_rule_overlay_assets};
use super::decode::validate_payload;
use super::encode::push_warning;
use super::format::{ShareError, ShareWarning};
use super::payload::{PreparedShareImport, SharePayload};

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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::models::OverlayOperation;
    use crate::sharing::build_payload;
    use crate::sharing::payload::fixtures::{click_overlay, overlay, preset, sample_rule};

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
            lifetime: crate::models::ResourceLifetime::Detach,
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
            lifetime: crate::models::ResourceLifetime::Detach,
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
            lifetime: crate::models::ResourceLifetime::Detach,
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
            lifetime: crate::models::ResourceLifetime::Detach,
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
}
