use egui::Ui;
use crate::gui::components::widgets::{self, Kind};
use crate::gui::theme::Tokens;
use crate::models::MacroEvent;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_footer(
    ui: &mut Ui,
    events: &mut [MacroEvent],
    repeat: &mut u32,
    speed: &mut f32,
    infinite: &mut bool,
    can_toggle_infinite: bool,
    group_full_presses: &mut bool,
    loc: &crate::i18n::Strings,
    t: &Tokens,
) {
    ui.horizontal_centered(|ui| {
        ui.add_space(16.0);

        // Repeat count (hidden when infinite mode is enabled)
        if !*infinite {
            widgets::hud(ui, loc.macro_studio_repeat_label, t.muted);
            ui.add(egui::DragValue::new(repeat).range(1..=10000).suffix("x"));
            ui.add_space(20.0);
        }

        widgets::hud(ui, loc.macro_studio_speed_label, t.muted);
        ui.add(egui::DragValue::new(speed).range(0.1..=10.0).speed(0.1).suffix("x"));

        ui.add_space(20.0);

        if !can_toggle_infinite {
            *infinite = false;
        }

        // Checkboxes group
        let inf_cb = egui::Checkbox::new(
            infinite,
            loc.macro_studio_checkbox_infinite,
        );
        let inf_tip = if !can_toggle_infinite {
            loc.macro_studio_infinite_disabled_tip
        } else {
            loc.macro_studio_infinite_tip
        };

        ui.add_enabled(can_toggle_infinite, inf_cb).on_hover_text(inf_tip);

        ui.add_space(16.0);

        ui.add_space(16.0);
        ui.checkbox(
            group_full_presses,
            loc.macro_studio_checkbox_group_presses,
        ).on_hover_text(loc.macro_studio_group_presses_tip);

        // Right side of footer: Holistic Humanize (Anti-Bot) toggle button (replacing Save & Close)
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(16.0);
            let has_humanize_targets = events.iter().any(|e| matches!(
                e,
                MacroEvent::Delay { .. } | MacroEvent::Text { .. } | MacroEvent::MouseMove { .. }
            ));
            let all_humanized = has_humanize_targets && events.iter().all(|e| match e {
                MacroEvent::Delay { organic, .. } => *organic,
                MacroEvent::Text { organic, .. } => *organic,
                MacroEvent::MouseMove { organic, .. } => *organic,
                _ => true,
            });
            let btn_kind = if all_humanized { Kind::Primary } else { Kind::Secondary };
            let btn_lbl = loc.macro_studio_checkbox_global_organic;
            let btn_w = widgets::button_width(ui, btn_lbl, 0.0);
            let btn_resp = widgets::button(ui, btn_lbl, btn_kind, btn_w)
                .on_hover_text(if all_humanized {
                    loc.macro_studio_global_organic_active_tip
                } else {
                    loc.macro_studio_global_organic_disabled_tip
                });

            if btn_resp.clicked() {
                let new_state = !all_humanized;
                for ev in events.iter_mut() {
                    match ev {
                        MacroEvent::Delay { organic, .. } => *organic = new_state,
                        MacroEvent::Text { organic, .. } => *organic = new_state,
                        MacroEvent::MouseMove { organic, .. } => *organic = new_state,
                        _ => {}
                    }
                }
            }
        });
    });
}
