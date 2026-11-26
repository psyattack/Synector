//! Share payload wire types: the decoded code, the import outcomes and the default version.

use serde::{Deserialize, Serialize};

use crate::models::{MacroPreset, OverlayDefinition, Rule};

use super::format::{ShareWarning, SHARE_VERSION};

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

#[cfg(test)]
pub(crate) mod fixtures {
    use crate::models::{
        Action, InteractiveControlKind, MacroPreset, OverlayClickAction, OverlayDefinition,
        OverlayElement, ProcessMatchMode, Rule, Trigger,
    };
    use crate::sharing::encode::compress_and_encode;

    use super::SharePayload;

    pub(crate) fn sample_rule(id: &str) -> Rule {
        Rule {
            id: id.into(),
            name: "Test Launch Rule".into(),
            description: "Launches game and kills bloat".into(),
            trigger: Some(Trigger::Process {
                process_name: "game.exe".into(),
                match_mode: ProcessMatchMode::Exact,
                single_instance: true,
                event: crate::models::ProcessTriggerEvent::Started,
            }),
            actions: vec![Action::StartProcess {
                file_path: "C:\\tools\\companion.exe".into(),
                arguments: "-silent".into(),
                working_directory: None,
                run_as_admin: false,
                only_if_not_running: true,
                run_hidden: true,
                lifetime: crate::models::ResourceLifetime::UntilTriggerProcessExit,
            }],
            custom_icon_path: Some("C:\\games\\game.exe".into()),
            auto_process_icon: true,
            ..Default::default()
        }
    }

    pub(crate) fn preset(id: &str) -> MacroPreset {
        MacroPreset {
            id: id.into(),
            name: format!("Preset {id}"),
            events: vec![],
            repeat: 1,
            speed: 1.0,
            infinite: false,
        }
    }

    pub(crate) fn overlay(id: &str) -> OverlayDefinition {
        let mut overlay = OverlayDefinition::default_new_overlay();
        overlay.id = id.into();
        overlay.name = format!("Overlay {id}");
        overlay
    }

    /// Bundled overlay with sender-local image and button icon paths.
    pub(crate) fn asset_overlay(id: &str) -> OverlayDefinition {
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
    pub(crate) fn click_overlay(id: &str, rule_id: &str) -> OverlayDefinition {
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

    pub(crate) fn encode(payload: &SharePayload) -> String {
        compress_and_encode(payload).expect("encode")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
