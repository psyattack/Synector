//! Shared preset picker body for the Overlay Studio and Macro Studio presets modals.
//!
//! The caller owns the modal shell (id, title, geometry), the preset data source, the row-detail
//! format and the effects of the returned intents (load/delete/save with its toast and close
//! logic); this module only renders the save-name field, the search field and the filtered list.

use egui::{vec2, CornerRadius, FontId, RichText, Sense};

use crate::gui::components::widgets::{self, Kind};
use crate::gui::theme::{self, Tokens};
use crate::i18n::Strings;

/// One preset row: the source index used to resolve the exact loaded preset, the id used for
/// the delete intent, the display name and the caller-formatted detail line.
pub(crate) struct PresetRow {
    pub index: usize,
    pub id: String,
    pub name: String,
    pub detail: String,
}

/// What the user asked for in the picker this frame. Each caller applies the intents with its
/// own load/delete/save semantics; `load_index` addresses the caller's source list so a
/// duplicate id (a hand-edited library) still loads the row that was clicked.
pub(crate) struct PresetPickerOutcome {
    pub save_requested: bool,
    pub load_index: Option<usize>,
    pub delete_id: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_preset_picker(
    ui: &mut egui::Ui,
    loc: &Strings,
    t: &Tokens,
    save_name: &mut String,
    search: &mut String,
    rows: Vec<PresetRow>,
    max_height: f32,
    scroll_salt: &str,
) -> PresetPickerOutcome {
    let mut save_requested = false;
    let mut load_index = None;
    let mut delete_id = None;

    // 1. Save Current as Preset Section (custom name input + save button)
    widgets::hud(ui, loc.presets_save_btn, t.muted);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let btn_w = widgets::button_width(ui, loc.presets_save_btn, 0.0);
        let text_w = (ui.available_width() - btn_w - ui.spacing().item_spacing.x).max(100.0);
        ui.add(
            egui::TextEdit::singleline(save_name)
                .hint_text(loc.presets_save_placeholder)
                .desired_width(text_w)
                .min_size(vec2(0.0, theme::CONTROL_H))
        );
        if widgets::button(ui, loc.presets_save_btn, Kind::Primary, btn_w).clicked() {
            save_requested = true;
        }
    });

    ui.add_space(12.0);
    widgets::rule(ui, 1.0);
    ui.add_space(10.0);

    // 2. Search (at top of list)
    ui.add(
        egui::TextEdit::singleline(search)
            .hint_text(loc.presets_search_placeholder)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::CONTROL_H - 4.0))
    );
    ui.add_space(10.0);

    // 3. Presets List
    let q = search.trim().to_lowercase();
    let filtered: Vec<_> = rows
        .into_iter()
        .filter(|row| q.is_empty() || row.name.to_lowercase().contains(&q) || row.id.to_lowercase().contains(&q))
        .collect();

    if filtered.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(loc.presets_empty).color(t.muted).size(13.0));
        });
    } else {
        egui::ScrollArea::vertical()
            .id_salt(scroll_salt)
            .max_height(max_height)
            .show(ui, |ui| {
                for row in &filtered {
                    ui.scope(|ui| {
                        let row_h = 52.0;
                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                        ui.painter().rect_filled(rect, CornerRadius::same(3), t.sunken);
                        ui.painter().rect_stroke(rect, CornerRadius::same(3), t.hairline(), egui::StrokeKind::Inside);

                        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // 1. Title and details on the left (first)
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(&row.name).font(FontId::new(13.0, theme::semibold())).color(t.ink_strong));
                                        ui.label(RichText::new(&row.detail).size(11.0).color(t.muted));
                                    });

                                    // 2. Action buttons on the right (second)
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if widgets::compact_button(ui, loc.presets_delete_btn, Kind::Danger, 24.0).clicked() {
                                            delete_id = Some(row.id.clone());
                                        }
                                        ui.add_space(4.0);
                                        if widgets::compact_button(ui, loc.presets_load_btn, Kind::Secondary, 24.0).clicked() {
                                            load_index = Some(row.index);
                                        }
                                    });
                                });
                            });
                        });
                    });
                    ui.add_space(4.0);
                }
            });
    }

    PresetPickerOutcome { save_requested, load_index, delete_id }
}
