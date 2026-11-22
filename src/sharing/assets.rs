//! Sender-local asset stripping and sensitive-action warnings shared by encode and import.

use crate::models::{Action, InteractiveControlKind, OverlayDefinition, OverlayElement, Rule};

use super::encode::push_warning;
use super::format::ShareWarning;

pub(super) fn collect_sensitive_warnings(rule: &Rule, warnings: &mut Vec<ShareWarning>) {
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
pub(super) fn strip_overlay_asset_paths(overlay: &mut OverlayDefinition) -> bool {
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
pub(super) fn strip_rule_overlay_assets(rule: &mut Rule) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::models::OverlayOperation;
    use crate::sharing::build_payload;
    use crate::sharing::payload::fixtures::{asset_overlay, sample_rule};
    use crate::sharing::{prepare_import, SharePayload};

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
}
