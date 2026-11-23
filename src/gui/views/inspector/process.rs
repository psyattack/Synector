//! Process inspector and process-name rows used across the editor.

use super::*;

/// Queues the force-terminate request. The daemon's answer is applied on a later frame: the
/// handler shows the toast and clears the selection only when the termination was confirmed.
pub(crate) fn terminate_process(state: &mut GuiState, proc: &ProcessSnapshotItem) {
    state.request_kill_process(proc.pid, &proc.name);
}

pub(crate) fn render_process_inspector(ui: &mut Ui, state: &mut GuiState, proc: &ProcessSnapshotItem, t: &Tokens) {
    let loc = state.loc();
    let scrollbar_w = ui.spacing().scroll.bar_width + ui.spacing().scroll.bar_inner_margin + ui.spacing().scroll.bar_outer_margin;
    let right_m = (20.0 - scrollbar_w).max(0.0).round() as i8;

    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 20,
            right: right_m,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 12.0;

            // Window Title
            widgets::hud(ui, loc.inspector_field_window_title, t.muted);
            let wtitle = if proc.window_title.is_empty() {
                loc.inspector_no_active_window
            } else {
                &proc.window_title
            };
            ui.label(
                egui::RichText::new(wtitle)
                    .family(theme::medium())
                    .size(13.0)
                    .color(t.ink),
            );

            // RAM Working Set
            let mb = proc.memory_bytes as f64 / 1_048_576.0;
            widgets::hud(ui, loc.inspector_field_working_set_ram, t.muted);
            ui.horizontal_wrapped(|ui| {
                widgets::badge(ui, &format!("{:.1} MB", mb), t.good);
                widgets::hud(ui, &format!("{} {}", proc.memory_bytes, loc.inspector_bytes_suffix), t.faint);
            });

            widgets::hrule(ui);

            widgets::eyebrow(ui, "02", loc.inspector_eyebrow_quick_actions);

            if widgets::button(ui, loc.inspector_btn_create_automation, Kind::Secondary, 0.0).clicked() {
                state.apply_process_to_draft_or_new_rule(&proc.name);
            }

            widgets::hrule(ui);

            // Terminating a process is destructive and cannot be undone: the first click only
            // arms the confirmation, and the selection is cleared only on a confirmed kill.
            let confirm_id = Id::unique(("inspector_terminate_confirm", proc.pid));
            let awaiting_confirm: bool = ui.data(|d| d.get_temp(confirm_id).unwrap_or(false));
            if awaiting_confirm {
                ui.label(egui::RichText::new(loc.inspector_terminate_confirm).size(11.5).color(t.warn));
                ui.horizontal(|ui| {
                    if widgets::button(ui, loc.inspector_btn_terminate_process, Kind::Danger, 0.0).clicked() {
                        ui.data_mut(|d| d.remove_temp::<bool>(confirm_id));
                        terminate_process(state, proc);
                    }
                    if widgets::button(ui, loc.inspector_btn_cancel, Kind::Secondary, 0.0).clicked() {
                        ui.data_mut(|d| d.remove_temp::<bool>(confirm_id));
                    }
                });
            } else if widgets::button(ui, loc.inspector_btn_terminate_process, Kind::Danger, 0.0).clicked() {
                ui.data_mut(|d| d.insert_temp(confirm_id, true));
            }
        });
    });
}

pub(crate) fn render_process_picker(ui: &mut Ui, loc: &crate::i18n::Strings) -> bool {
    widgets::button(ui, loc.inspector_pick_running, Kind::Secondary, 0.0)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// One process-name row: right-aligned picker button followed by the editable name.
/// Returns `true` when the picker was clicked. The caller draws its own HUD label.
pub(crate) fn render_process_name_row(
    ui: &mut Ui,
    loc: &crate::i18n::Strings,
    process_name: &mut String,
    hint: &str,
) -> bool {
    let mut picked = false;
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            picked = render_process_picker(ui, loc);
            ui.add(
                egui::TextEdit::singleline(process_name)
                    .hint_text(hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::control_height(ui)))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            );
        });
    });
    picked
}

/// Optional-string variant of [`render_process_name_row`]: an empty (or whitespace-only)
/// name clears the field, and the draft is written only when the text actually changed.
pub(crate) fn render_optional_process_name_row(
    ui: &mut Ui,
    loc: &crate::i18n::Strings,
    process_name: &mut Option<String>,
    hint: &str,
) -> bool {
    let mut buffer = process_name.clone().unwrap_or_default();
    let mut picked = false;
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            picked = render_process_picker(ui, loc);
            if ui.add(
                egui::TextEdit::singleline(&mut buffer)
                    .hint_text(hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::control_height(ui)))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            ).changed() {
                let trimmed = buffer.trim();
                *process_name = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };
            }
        });
    });
    picked
}
