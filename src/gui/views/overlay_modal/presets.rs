//! Overlay Studio presets: slug/id derivation, saving the current studio overlay as a library
//! preset and the presets modal shell around the shared picker.

use egui::vec2;

use crate::gui::components::modal::{render_modal_header, Modal, ModalHeader};
use crate::gui::components::widgets;
use crate::gui::state::GuiState;
use crate::gui::theme::Tokens;
use crate::gui::views::presets::{render_preset_picker, PresetRow};
use crate::models::OverlayDefinition;

use super::clamp_elements_to_canvas;

/// Library id for a typed preset name, e.g. `"Gaming Crosshair"` -> `overlay_gaming_crosshair`.
/// Returns `None` when the name has no alphanumeric characters: saving `"!!!"` used to produce
/// the id `overlay_`, which is not a preset (U02-006).
fn preset_slug(name: &str) -> Option<String> {
    let clean: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let trimmed = clean.trim_matches('_');
    (!trimmed.is_empty()).then(|| format!("overlay_{trimmed}"))
}

/// Ids and names already present in the target library.
fn library_overlay_keys(overlays: &[OverlayDefinition]) -> Vec<(&str, &str)> {
    overlays.iter().map(|o| (o.id.as_str(), o.name.as_str())).collect()
}

/// Picks the id a preset is saved under. A derived id that is already taken — or a name that
/// already exists in the library — gets `_2`, `_3`, … instead of overwriting the existing
/// preset, so a save can never silently replace another, built-in overlay (U02-006).
fn unique_preset_id(base: &str, name: &str, existing: &[(&str, &str)]) -> String {
    let base_taken = existing
        .iter()
        .any(|(id, existing_name)| *id == base || existing_name.eq_ignore_ascii_case(name));
    if !base_taken {
        return base.to_string();
    }
    for n in 2..=9999 {
        let candidate = format!("{base}_{n}");
        if !existing.iter().any(|(id, _)| *id == candidate) {
            return candidate;
        }
    }
    format!("{base}_{}", existing.len() + 2)
}

/// Saves the studio overlay as a new library preset. The id is derived from the typed name, but
/// an existing id or name gets a numeric suffix instead of being silently replaced, and a name
/// without alphanumerics is rejected with a toast (U02-006).
fn save_studio_overlay_preset(state: &mut GuiState, loc: &crate::i18n::Strings) {
    let Some(studio) = state.overlay_studio.as_ref() else { return };
    let name = if studio.preset_save_name.trim().is_empty() {
        if studio.overlay.name.trim().is_empty() {
            loc.overlay_default_preset_name.to_string()
        } else {
            studio.overlay.name.trim().to_string()
        }
    } else {
        studio.preset_save_name.trim().to_string()
    };
    let Some(base_id) = preset_slug(&name) else {
        crate::logging::warn("CONFIG", "Overlay preset save rejected: the name has no usable characters");
        state.toast = Some((
            loc.presets_save_invalid_name.to_string(),
            std::time::Instant::now(),
        ));
        return;
    };

    let library_overlays = state
        .daemon_state
        .as_ref()
        .map(|s| s.config.overlays.clone())
        .unwrap_or_else(|| crate::config::Library::read().overlays);
    let keys = library_overlay_keys(&library_overlays);
    let fresh_id = unique_preset_id(&base_id, &name, &keys);

    let mut saved_def = studio.overlay.clone();
    saved_def.id = fresh_id;
    saved_def.name = name.clone();

    state.request_save_overlay_preset(saved_def, name);
}

pub fn render_overlay_presets_modal(ctx: &egui::Context, state: &mut GuiState) {
    let t = Tokens::get(ctx);
    let is_open = state.overlay_studio.as_ref().is_some_and(|s| s.show_presets_popover);
    let loc = state.loc();
    let mut save_requested = false;
    let mut delete_requested = None;

    let modal_res = Modal::new("overlay-presets-modal", is_open)
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
                    |ui| { widgets::hud(ui, loc.overlay_studio_presets_title, t.ink_strong); },
                    |_| {},
                );
            });

            // Body
            frame.body(|ui| {
                let Some(studio) = state.overlay_studio.as_mut() else { return; };
                egui::Frame::new().inner_margin(egui::Margin::same(16)).show(ui, |ui| {
                    let all_presets = state.daemon_state.as_ref()
                        .map(|s| s.config.overlays.clone())
                        .unwrap_or_else(|| crate::config::Library::read().overlays);
                    let rows: Vec<PresetRow> = all_presets.iter()
                        .enumerate()
                        .map(|(index, p)| PresetRow {
                            index,
                            id: p.id.clone(),
                            name: p.name.clone(),
                            detail: format!(
                                "{} {}, {}×{}",
                                p.elements.len(),
                                loc.presets_badge_elements,
                                p.rect.width as i32,
                                p.rect.height as i32
                            ),
                        })
                        .collect();

                    let outcome = render_preset_picker(
                        ui,
                        loc,
                        &t,
                        &mut studio.preset_save_name,
                        &mut studio.preset_search_query,
                        rows,
                        340.0,
                        "overlay_presets_scroll",
                    );
                    save_requested = outcome.save_requested;

                    if let Some(index) = outcome.load_index {
                        if let Some(loaded) = all_presets.get(index).cloned() {
                            studio.push_undo_snapshot();
                            studio.overlay.elements = loaded.elements;
                            studio.overlay.style = loaded.style;
                            studio.overlay.rect = loaded.rect;
                            studio.overlay.anchor = loaded.anchor;
                            studio.overlay.opacity = loaded.opacity;
                            studio.overlay.click_through = loaded.click_through;
                            studio.overlay.draggable = loaded.draggable;
                            clamp_elements_to_canvas(&mut studio.overlay);
                            studio.show_presets_popover = false;
                            state.toast = Some((loc.presets_loaded_toast.replace("{}", &loaded.name), std::time::Instant::now()));
                        }
                    }

                    delete_requested = outcome.delete_id;
                });
            });
        });

    if save_requested {
        save_studio_overlay_preset(state, loc);
    }

    if let Some(del_id) = delete_requested {
        state.request_delete_overlay_preset(&del_id);
    }

    if modal_res.is_some_and(|r| r.close_requested) {
        if let Some(studio) = state.overlay_studio.as_mut() {
            studio.show_presets_popover = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- U02-006: preset ids ----------------

    #[test]
    fn test_preset_slug_rejects_punctuation_only_name() {
        assert_eq!(preset_slug("Crosshair").as_deref(), Some("overlay_crosshair"));
        assert_eq!(preset_slug("  Gaming Crosshair  ").as_deref(), Some("overlay_gaming_crosshair"));
        assert_eq!(preset_slug("!!!"), None);
        assert_eq!(preset_slug("   "), None);
        assert_eq!(preset_slug("__"), None);
    }

    #[test]
    fn test_preset_id_collision_gets_unique_suffix() {
        let existing = [("overlay_crosshair", "Gaming Crosshair"), ("overlay_telemetry", "System Telemetry")];

        // A free id is used verbatim.
        assert_eq!(unique_preset_id("overlay_hud", "HUD", &existing), "overlay_hud");

        // A built-in id is never silently replaced: the new preset gets a suffix.
        assert_eq!(unique_preset_id("overlay_crosshair", "Crosshair", &existing), "overlay_crosshair_2");

        // Already-suffixed ids keep counting up.
        let with_suffix = [("overlay_crosshair", "x"), ("overlay_crosshair_2", "y")];
        assert_eq!(unique_preset_id("overlay_crosshair", "Crosshair", &with_suffix), "overlay_crosshair_3");

        // A name collision is handled the same way, and re-running the same name always returns
        // instead of looping forever on the taken name.
        for _ in 0..5 {
            assert_eq!(unique_preset_id("overlay_telemetry", "System Telemetry", &existing), "overlay_telemetry_2");
        }
    }
}
