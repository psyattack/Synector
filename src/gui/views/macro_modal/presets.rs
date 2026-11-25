use eframe::emath::vec2;
use crate::gui::components::modal::{render_modal_header, Modal, ModalHeader};
use crate::gui::components::widgets;
use crate::gui::state::GuiState;
use crate::gui::theme::Tokens;
use crate::gui::views::presets::{render_preset_picker, PresetRow};

/// Identity of a macro preset is independent of its display name. Editing an existing preset
/// reuses its id (which is what makes a rename possible); saving under an already used display
/// name updates that preset; otherwise a slug-based id is made unique against the existing
/// presets, because `SaveMacroPreset` replaces by id and two different names that normalize
/// identically would otherwise silently overwrite each other.
fn allocate_macro_preset_id(
    name: &str,
    edited_id: Option<&str>,
    existing: &[crate::models::MacroPreset],
) -> String {
    if let Some(id) = edited_id.filter(|id| !id.trim().is_empty()) {
        return id.to_string();
    }
    if let Some(preset) = existing.iter().find(|preset| preset.name.eq_ignore_ascii_case(name)) {
        return preset.id.clone();
    }
    let slug = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>();
    let base = format!("macro_{}", slug.trim_matches('_'));
    let base = if base == "macro_" { "macro_preset".to_string() } else { base };
    let taken = |candidate: &str| existing.iter().any(|preset| preset.id == candidate);
    if !taken(&base) {
        return base;
    }
    let mut suffix = 2u32;
    loop {
        let candidate = format!("{base}_{suffix}");
        if !taken(&candidate) {
            return candidate;
        }
        suffix = suffix.saturating_add(1);
    }
}

pub fn render_macro_presets_modal(ctx: &egui::Context, state: &mut GuiState) {
    let t = Tokens::get(ctx);
    let is_open = state.show_macro_presets_modal;
    let loc = state.loc();
    let mut save_requested = false;

    let modal_res = Modal::new("macro-presets-modal", is_open)
        .width_fraction(0.55)
        .height_fraction(0.68)
        .min_size(vec2(540.0, 480.0))
        .max_size(vec2(640.0, 640.0))
        .header_height(46.0)
        .header_sunken(true)
        .show(ctx, &t, |frame| {
            // Header: Title on left, Close on right
            frame.header(|ui, close_req| {
                render_modal_header(
                    ui,
                    close_req,
                    &t,
                    ModalHeader {
                        glyph: Some(egui_phosphor::regular::BOOKMARK_SIMPLE),
                        glyph_size: 16.0,
                        close_tip: loc.modal_tip_close,
                        left_pad: 16.0,
                        glyph_pad: 8.0,
                        right_pad: 14.0,
                        item_spacing_x: None,
                    },
                    |ui| { widgets::hud(ui, loc.macro_studio_presets_title, t.ink_strong); },
                    |_| {},
                );
            });

            // Body
            frame.body(|ui| {
                egui::Frame::new().inner_margin(egui::Margin::same(16)).show(ui, |ui| {
                    let all_presets = state.daemon_state.as_ref()
                        .map(|s| s.config.macro_presets.clone())
                        .unwrap_or_else(|| crate::config::Library::read().macro_presets);
                    let rows: Vec<PresetRow> = all_presets.iter()
                        .enumerate()
                        .map(|(index, p)| {
                            let inf_or_rep = if p.infinite {
                                format!(", {}", loc.macro_studio_infinite_tip)
                            } else if p.repeat > 1 {
                                format!(", {}x", p.repeat)
                            } else {
                                String::new()
                            };
                            PresetRow {
                                index,
                                id: p.id.clone(),
                                name: p.name.clone(),
                                detail: format!("{} {}, {:.1}x speed{}", p.events.len(), loc.presets_badge_events, p.speed, inf_or_rep),
                            }
                        })
                        .collect();

                    let outcome = render_preset_picker(
                        ui,
                        loc,
                        &t,
                        &mut state.macro_preset_save_name,
                        &mut state.macro_preset_search_query,
                        rows,
                        360.0,
                        "macro_presets_scroll",
                    );
                    save_requested = outcome.save_requested;

                    if let Some(index) = outcome.load_index {
                        if let Some(loaded) = all_presets.get(index).cloned() {
                            if let Some(r) = state.editing_rule.as_mut() {
                                if let Some(edit_info) = state.macro_editor.as_ref() {
                                    if let Some(crate::models::Action::Macro { events, repeat, speed, infinite, preset_id, .. }) = r.actions.get_mut(edit_info.step_idx) {
                                        *events = loaded.events;
                                        *repeat = loaded.repeat;
                                        *speed = loaded.speed;
                                        *infinite = loaded.infinite;
                                        *preset_id = None;
                                    }
                                }
                            }
                            state.show_macro_presets_modal = false;
                            state.toast = Some((loc.presets_loaded_toast.replace("{}", &loaded.name), std::time::Instant::now()));
                        }
                    }

                    if let Some(del_id) = outcome.delete_id {
                        state.request_delete_macro_preset(&del_id);
                    }
                });
            });
        });

    if save_requested {
        let current_macro = state.editing_rule.as_ref().and_then(|r| {
            let edit_info = state.macro_editor.as_ref()?;
            if let Some(crate::models::Action::Macro { events, repeat, speed, infinite, preset_id, .. }) = r.actions.get(edit_info.step_idx) {
                let name = if state.macro_preset_save_name.trim().is_empty() {
                    if r.name.trim().is_empty() { loc.macro_studio_custom_preset_name.to_string() } else { r.name.trim().to_string() }
                } else {
                    state.macro_preset_save_name.trim().to_string()
                };
                Some((name, preset_id.clone(), events.clone(), *repeat, *speed, *infinite))
            } else {
                None
            }
        });

        if let Some((name, edited_preset_id, events, repeat, speed, infinite)) = current_macro {
            // The daemon replaces presets by id, so allocate against the list this modal renders
            // (or the library fallback when the daemon is offline) instead of assuming the name
            // is unique.
            let existing: Vec<crate::models::MacroPreset> = state
                .daemon_state
                .as_ref()
                .map(|ds| ds.config.macro_presets.clone())
                .unwrap_or_else(|| crate::config::Library::read().macro_presets.clone());
            let saved_preset = crate::models::MacroPreset {
                id: allocate_macro_preset_id(&name, edited_preset_id.as_deref(), &existing),
                name: name.clone(),
                events,
                repeat,
                speed,
                infinite,
            };

            state.request_save_macro_preset(saved_preset, name);
        }
    }

    if modal_res.is_some_and(|r| r.close_requested) {
        state.show_macro_presets_modal = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// U03-011: preset identity no longer follows the display name. Editing keeps the id (so a
    /// rename updates the preset), a repeated name updates the same preset, and two different
    /// names that normalize identically get distinct ids instead of overwriting each other.
    #[test]
    fn test_allocate_macro_preset_id_keeps_identity_and_avoids_collisions() {
        let mut existing = vec![
            crate::models::MacroPreset::new("macro_a_b", "A/B", Vec::new()),
            crate::models::MacroPreset::new("macro_a_b_2", "A_B", Vec::new()),
        ];

        // Editing an existing preset reuses its id even when the name changed.
        assert_eq!(allocate_macro_preset_id("Renamed", Some("macro_a_b_2"), &existing), "macro_a_b_2");
        // The same display name updates the existing preset instead of creating a duplicate.
        assert_eq!(allocate_macro_preset_id("A_B", None, &existing), "macro_a_b_2");
        // A different name that normalizes identically gets a fresh, non-colliding id.
        assert_eq!(allocate_macro_preset_id("A B", None, &existing), "macro_a_b_3");

        let first = allocate_macro_preset_id("C/D", None, &existing);
        existing.push(crate::models::MacroPreset::new(first.clone(), "C/D", Vec::new()));
        let second = allocate_macro_preset_id("C D", None, &existing);
        assert_ne!(first, second, "different display names must not share an id");

        assert_eq!(allocate_macro_preset_id("!!!", None, &[]), "macro_preset");
        assert_eq!(allocate_macro_preset_id("Fresh", None, &existing), "macro_fresh");
    }
}
