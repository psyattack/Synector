use egui::{Sense, Stroke, Ui, vec2};
use crate::gui::components::widgets;
use crate::gui::theme::{self, Tokens};
use crate::i18n::Strings;
use crate::models::{Action, ConditionOperator, Rule};

use super::summary::{
    action_icon_glyph, format_action_brief, format_single_condition_brief,
    format_single_condition_tooltip, format_trigger_display,
};

pub(super) fn render_track_pipeline(
    ui: &mut Ui,
    loc: &Strings,
    rule: &Rule,
    exec_opt: Option<crate::ipc::ActiveRuleExecutionState>,
    is_test_running: bool,
    t: &Tokens,
) {
    ui.spacing_mut().item_spacing.x = 6.0;

    // 0. Live RUNNING Badge before Trigger
    let is_running = exec_opt.is_some() || is_test_running;
    if is_running {
        let running_text = if let Some(exec) = &exec_opt {
            if exec.active_count > 1 {
                format!("{} {}x", loc.rules_badge_running, exec.active_count)
            } else {
                loc.rules_badge_running.to_string()
            }
        } else {
            loc.rules_badge_running.to_string()
        };
        widgets::badge(ui, &running_text, t.good);
    }

    // 1. Trigger Chip (No "TRIGGER:" prefix! Clean glyph and detail)
    let (trig_glyph, trig_label) = format_trigger_display(&rule.trigger, loc);
    let trig_chip = format!("{} {}", trig_glyph, trig_label);
    let galley = widgets::caps(ui, &trig_chip, 9.5, theme::mono(), 0.5, t.accent_ink);
    let chip_w = galley.size().x + 12.0;
    let (chip_rect, _chip_resp) = ui.allocate_exact_size(vec2(chip_w, 22.0), Sense::hover());
    ui.painter().rect(chip_rect, t.chip_corner(), t.sunken, t.hairline(), egui::StrokeKind::Inside);
    widgets::paint_centred(ui, chip_rect, galley, t.accent_ink);

    // 2. Pre-conditions (if any) - NO arrow before IF
    if !rule.conditions.is_empty() {
        widgets::hud(ui, loc.rules_summary_cond_if, t.faint);
        for (c_idx, item) in rule.conditions.iter().enumerate() {
            if c_idx > 0 {
                let op_str = match item.operator {
                    ConditionOperator::And => loc.inspector_cond_op_and,
                    ConditionOperator::Or => loc.inspector_cond_op_or,
                };
                widgets::hud(ui, op_str, t.faint);
            }
            let label = format_single_condition_brief(&item.condition, loc);
            let tip = format_single_condition_tooltip(&item.condition, c_idx + 1, loc);
            widgets::badge(ui, &label, t.faint).on_hover_text(tip);
        }
    }

    // 3. Arrow divider to actions
    widgets::hud(ui, "➔", t.faint);

    // 4. Action Steps (No step numbering! The running step is highlighted only.)
    if rule.actions.is_empty() {
        widgets::hud(ui, loc.rules_summary_no_actions, t.muted);
    } else {
        for (a_idx, action) in rule.actions.iter().enumerate() {
            if a_idx > 0 {
                widgets::hud(ui, "➔", t.faint);
            }

            let is_current = exec_opt.is_some_and(|e| e.current_step == a_idx);
            render_action_chip(ui, action, is_current, t.ink, loc, t);
        }
    }

    // 5. Companion mode
    let has_companion = rule.actions.iter().any(|a| matches!(a, Action::StartProcess { close_on_exit: true, .. }));
    if has_companion {
        widgets::badge(ui, loc.rules_summary_bound_satellite, t.good);
    }
}

/// One action-step chip.
fn render_action_chip(
    ui: &mut Ui,
    action: &Action,
    is_current: bool,
    ink: egui::Color32,
    loc: &Strings,
    t: &Tokens,
) {
    let brief = format_action_brief(action, loc);
    let icon = action_icon_glyph(action);

    let (chip_fill, chip_stroke, chip_ink, font) = if is_current {
        (
            t.accent.gamma_multiply(0.35),
            Stroke::new(1.5, t.accent_ink),
            t.accent_ink,
            theme::mono_bold(),
        )
    } else {
        (t.sunken, t.hairline(), ink, theme::mono())
    };

    let step_label = format!("{icon} {brief}");
    let galley = widgets::caps(ui, &step_label, 9.5, font, 0.5, chip_ink);
    let chip_w = galley.size().x + 12.0;
    let (chip_rect, chip_resp) = ui.allocate_exact_size(vec2(chip_w, 22.0), Sense::hover());
    ui.painter().rect(chip_rect, t.chip_corner(), chip_fill, chip_stroke, egui::StrokeKind::Inside);
    widgets::paint_centred(ui, chip_rect, galley, chip_ink);

    if is_current {
        chip_resp.on_hover_text(loc.rules_summary_step_active);
    }
}
