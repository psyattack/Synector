//! Overlay Studio view model and the open/close paths that write the edited definition back
//! into the rule action or the overlay library.

use crate::ipc::IpcRequest;

use super::ipc::{DirectLibraryChange, DirectWrite, IpcJobKind};
use super::GuiState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorPickerTarget {
    CardBg,
    CardBorder,
    ElementPrimary(usize),
    ElementSecondary(usize),
    ElementTertiary(usize),
}

#[derive(Debug, Clone)]
pub struct OverlayStudioState {
    pub overlay: crate::models::OverlayDefinition,
    pub is_calibrating: bool,
    pub selected_element_idx: Option<usize>,
    pub action_target_idx: Option<usize>,
    pub is_exit_pipeline: bool,
    pub active_color_target: Option<ColorPickerTarget>,
    pub snap_to_grid: bool,
    pub card_lock_aspect: bool,
    pub canvas_pan: egui::Vec2,
    pub canvas_zoom: Option<f32>,
    pub last_synced_pos: Option<(f32, f32)>,
    pub undo_stack: Vec<crate::models::OverlayDefinition>,
    pub redo_stack: Vec<crate::models::OverlayDefinition>,
    pub show_presets_popover: bool,
    pub preset_save_name: String,
    pub preset_search_query: String,
}

impl OverlayStudioState {
    /// Fresh studio for one overlay definition. `action_target_idx` marks an edit that is
    /// written back into the rule action instead of the library.
    pub fn new(
        overlay: crate::models::OverlayDefinition,
        action_target_idx: Option<usize>,
        is_exit_pipeline: bool,
        preset_save_name: String,
    ) -> Self {
        Self {
            overlay,
            is_calibrating: false,
            selected_element_idx: None,
            action_target_idx,
            is_exit_pipeline,
            active_color_target: None,
            snap_to_grid: false,
            card_lock_aspect: false,
            canvas_pan: egui::Vec2::ZERO,
            canvas_zoom: None,
            last_synced_pos: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            show_presets_popover: false,
            preset_save_name,
            preset_search_query: String::new(),
        }
    }
}

impl GuiState {
    pub fn open_overlay_studio(&mut self, overlay_id: &str) {
        let library = crate::config::Library::read();
        let overlay = library
            .overlays
            .iter()
            .find(|o| o.id == overlay_id)
            .cloned()
            .unwrap_or_else(|| default_overlay_for(overlay_id));
        self.show_overlay_studio(OverlayStudioState::new(overlay, None, false, String::new()));
    }

    pub fn open_overlay_studio_for_action(&mut self, action_idx: usize, is_exit: bool) {
        let studio = {
            let Some(rule) = &self.editing_rule else { return };
            let actions = if is_exit { &rule.on_exit_actions } else { &rule.actions };
            let Some(crate::models::Action::Overlay { overlay_id, custom_overlay, .. }) = actions.get(action_idx) else {
                return;
            };
            let overlay = if let Some(cust) = custom_overlay {
                (**cust).clone()
            } else {
                let library = crate::config::Library::read();
                library
                    .overlays
                    .iter()
                    .find(|o| o.id == *overlay_id)
                    .cloned()
                    .unwrap_or_else(|| default_overlay_for(overlay_id))
            };
            let preset_save_name = overlay.name.clone();
            OverlayStudioState::new(overlay, Some(action_idx), is_exit, preset_save_name)
        };
        self.show_overlay_studio(studio);
    }

    fn show_overlay_studio(&mut self, studio: OverlayStudioState) {
        self.overlay_studio = Some(studio.clone());
        self.last_overlay_studio = Some(studio);
    }

    pub fn close_overlay_studio(&mut self) {
        let Some(studio) = self.overlay_studio.as_ref() else { return };
        let overlay = studio.overlay.clone();
        let action_target = studio.action_target_idx;
        let is_exit_pipeline = studio.is_exit_pipeline;
        self.request_hide_overlay_preview(&overlay.id);
        let mut save_to_library = action_target.is_none();
        if let Some(action_idx) = action_target {
            if let Some(rule) = &mut self.editing_rule {
                let actions = if is_exit_pipeline {
                    &mut rule.on_exit_actions
                } else {
                    &mut rule.actions
                };
                if let Some(crate::models::Action::Overlay { custom_overlay, overlay_id, .. }) = actions.get_mut(action_idx) {
                    if custom_overlay.is_some() {
                        // Inline definitions live in the rule action, not in the library.
                        *custom_overlay = Some(Box::new(overlay.clone()));
                        *overlay_id = overlay.id.clone();
                    } else {
                        save_to_library = true;
                    }
                }
            }
        }
        if save_to_library {
            let direct = DirectWrite::Library(DirectLibraryChange::UpsertOverlay(Box::new(overlay.clone())));
            self.enqueue_ipc(
                IpcJobKind::SaveOverlay { overlay_id: overlay.id.clone() },
                IpcRequest::SaveOverlay(overlay),
                Some(direct),
            );
        }
        self.overlay_studio = None;
    }
}

/// Built-in overlay definitions by id; unknown ids fall back to the blank template. Shared by
/// both studio openers so a new built-in overlay is declared once.
fn default_overlay_for(id: &str) -> crate::models::OverlayDefinition {
    match id {
        "overlay_crosshair" => crate::models::OverlayDefinition::default_crosshair(),
        "overlay_telemetry" => crate::models::OverlayDefinition::default_telemetry(),
        "overlay_macro_badge" => crate::models::OverlayDefinition::default_macro_badge(),
        _ => crate::models::OverlayDefinition::default_new_overlay(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_overlay_for_builtin_ids() {
        assert_eq!(
            default_overlay_for("overlay_crosshair").id,
            crate::models::OverlayDefinition::default_crosshair().id
        );
        assert_eq!(
            default_overlay_for("overlay_telemetry").id,
            crate::models::OverlayDefinition::default_telemetry().id
        );
        assert_eq!(
            default_overlay_for("overlay_macro_badge").id,
            crate::models::OverlayDefinition::default_macro_badge().id
        );
        assert_eq!(
            default_overlay_for("something_custom").id,
            crate::models::OverlayDefinition::default_new_overlay().id
        );

        let studio = OverlayStudioState::new(
            crate::models::OverlayDefinition::default_crosshair(),
            Some(3),
            true,
            "Saved name".into(),
        );
        assert_eq!(studio.action_target_idx, Some(3));
        assert!(studio.is_exit_pipeline);
        assert_eq!(studio.preset_save_name, "Saved name");
        assert!(studio.undo_stack.is_empty() && studio.redo_stack.is_empty());
        assert!(!studio.is_calibrating);
    }
}
