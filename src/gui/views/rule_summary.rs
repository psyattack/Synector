//! Glyph helpers shared by the rules list and the inspector: the rule-card fallback icon and the
//! condition-row icon. Presentation lives in the GUI, not in the model.

use crate::gui::views::rules_view::action_icon_glyph;
use crate::models::{Rule, RuleCondition, Trigger};

/// Icon shown for one condition row. Presentation lives in the GUI, not in the model.
pub(crate) fn condition_icon_glyph(condition: &RuleCondition) -> &'static str {
    match condition {
        RuleCondition::ProcessRunning { .. } => egui_phosphor::regular::CPU,
        RuleCondition::WindowFocused { .. } => egui_phosphor::regular::APP_WINDOW,
        RuleCondition::TimeRange { .. } => egui_phosphor::regular::CLOCK,
        RuleCondition::Event { .. } => egui_phosphor::regular::BELL,
        RuleCondition::ResourceThreshold { .. } => egui_phosphor::regular::CHART_LINE,
        RuleCondition::FileWatcher { .. } => egui_phosphor::regular::FILE,
        RuleCondition::RuleRunning { .. } => egui_phosphor::regular::PLAY,
    }
}

/// Icon shown for a rule card: the first action decides, then the trigger, then a gear.
/// This is the single action-to-glyph table; `models` deliberately carries no presentation.
pub(crate) fn default_icon_glyph(rule: &Rule) -> &'static str {
    if let Some(action) = rule.actions.first() {
        return action_icon_glyph(action);
    }
    match &rule.trigger {
        Some(Trigger::Manual) => egui_phosphor::regular::PLAY,
        Some(Trigger::Process { .. }) => egui_phosphor::regular::CPU,
        Some(Trigger::WindowFocus { .. }) => egui_phosphor::regular::EYE,
        Some(Trigger::ResourceThreshold { .. }) => egui_phosphor::regular::CHART_LINE,
        Some(Trigger::GlobalHotkey { .. }) => egui_phosphor::regular::COMMAND,
        Some(Trigger::SystemEvent { .. }) => egui_phosphor::regular::LIGHTNING,
        Some(Trigger::Schedule { .. }) => egui_phosphor::regular::CALENDAR,
        Some(Trigger::FileWatcher { .. }) => egui_phosphor::regular::FOLDER_NOTCH_OPEN,
        Some(Trigger::Webhook { .. }) => egui_phosphor::regular::GLOBE_HEMISPHERE_WEST,
        None => egui_phosphor::regular::GEAR,
    }
}
