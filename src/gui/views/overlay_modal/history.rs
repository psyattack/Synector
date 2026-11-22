//! Overlay studio history: the keyboard history shortcuts, the element shortcut set, the
//! one-snapshot-per-inspector-session bookkeeping and the studio undo/redo actions.

use egui::Ui;


use crate::gui::state::OverlayStudioState;
use crate::gui::theme::Tokens;
use crate::models::{OverlayDefinition, OverlayElement};

use super::clamp_elements_to_canvas;
use super::geometry::clamp_crosshair_coord;
use super::get_element_size;
use super::inspector::render_overlay_inspector;

#[cfg(windows)]
static OVERLAY_STUDIO_LAST_Z: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(windows)]
static OVERLAY_STUDIO_LAST_Y: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StudioHistoryAction {
    Undo,
    Redo,
}

/// Ctrl+Z / Ctrl+Y history input for the studio. Uses physical Win32 `GetAsyncKeyState` edge
/// transitions (immune to layout like Russian Cyrillic) with an egui fallback, and consumes
/// the keys so the rest of the UI never sees them.
pub(super) fn read_studio_history_shortcuts(
    ctx: &egui::Context,
    is_open: bool,
    is_typing: bool,
) -> Option<StudioHistoryAction> {
    if !is_open {
        #[cfg(windows)]
        {
            use std::sync::atomic::Ordering;
            OVERLAY_STUDIO_LAST_Z.store(false, Ordering::Relaxed);
            OVERLAY_STUDIO_LAST_Y.store(false, Ordering::Relaxed);
        }
        return None;
    }

    let mut do_undo = false;
    let mut do_redo = false;

    #[cfg(windows)]
    if !is_typing && ctx.input(|i| i.focused) {
        use std::sync::atomic::Ordering;
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

        let ctrl_down = unsafe { (GetAsyncKeyState(0x11) as u16 & 0x8000) != 0 }; // VK_CONTROL
        let shift_down = unsafe { (GetAsyncKeyState(0x10) as u16 & 0x8000) != 0 }; // VK_SHIFT
        let z_down = unsafe { (GetAsyncKeyState(0x5A) as u16 & 0x8000) != 0 }; // 'Z'
        let y_down = unsafe { (GetAsyncKeyState(0x59) as u16 & 0x8000) != 0 }; // 'Y'

        let was_z = OVERLAY_STUDIO_LAST_Z.swap(z_down, Ordering::SeqCst);
        let was_y = OVERLAY_STUDIO_LAST_Y.swap(y_down, Ordering::SeqCst);

        if ctrl_down {
            if z_down && !was_z {
                if shift_down {
                    do_redo = true;
                } else {
                    do_undo = true;
                }
            } else if y_down && !was_y {
                do_redo = true;
            }
        }
    }

    ctx.input(|i| {
        let is_ctrl = i.modifiers.command || i.modifiers.ctrl;
        let is_shift = i.modifiers.shift;
        for event in &i.events {
            if let egui::Event::Key { key, physical_key, pressed: true, .. } = event {
                let is_z = *key == egui::Key::Z || *physical_key == Some(egui::Key::Z);
                let is_y = *key == egui::Key::Y || *physical_key == Some(egui::Key::Y);
                if is_ctrl && !is_shift && is_z {
                    do_undo = true;
                } else if (is_ctrl && is_y) || (is_ctrl && is_shift && is_z) {
                    do_redo = true;
                }
            }
        }
    });

    // Also consume standard keys via consume_key
    ctx.input_mut(|i| {
        if i.consume_key(egui::Modifiers::CTRL, egui::Key::Z) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z) {
            do_undo = true;
        }
        if i.consume_key(egui::Modifiers::CTRL, egui::Key::Y) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
            || i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Z)
            || i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z)
        {
            do_redo = true;
        }
    });

    if do_undo {
        Some(StudioHistoryAction::Undo)
    } else if do_redo {
        Some(StudioHistoryAction::Redo)
    } else {
        None
    }
}

/// Tab cycling, Delete, Ctrl+D, arrow nudges, layer reordering and Escape for the selected
/// element. Consumes its keys so egui widgets never see them.
pub(super) fn handle_studio_element_shortcuts(
    ctx: &egui::Context,
    studio: &mut OverlayStudioState,
    is_open: bool,
    is_typing: bool,
) {
    if !(is_open && !is_typing) {
        if studio.active_color_target.is_some() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            studio.active_color_target = None;
        }
        return;
    }

    // Tab / Shift+Tab -> Cycle selected element forward / backward
    let mut tab_nav: Option<bool> = None;
    ctx.input(|i| {
        for event in &i.events {
            if let egui::Event::Key { key: egui::Key::Tab, pressed: true, modifiers, .. } = event {
                tab_nav = Some(!modifiers.shift);
            }
        }
    });

    if let Some(forward) = tab_nav {
        ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab);
            i.consume_key(egui::Modifiers::NONE, egui::Key::Tab);
        });
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
            m.move_focus(egui::FocusDirection::None);
        });
        let total = studio.overlay.elements.len();
        if total > 0 {
            studio.selected_element_idx = match studio.selected_element_idx {
                Some(cur) => {
                    let cur = cur.min(total - 1);
                    if forward {
                        Some((cur + 1) % total)
                    } else {
                        Some((cur + total - 1) % total)
                    }
                }
                None => {
                    if forward {
                        Some(0)
                    } else {
                        Some(total - 1)
                    }
                }
            };
            ctx.request_repaint();
        }
    }

    // Delete or Backspace -> remove selected element
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete) || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)) {
        if let Some(sel) = studio.selected_element_idx {
            if sel < studio.overlay.elements.len() {
                studio.push_undo_snapshot();
                studio.overlay.elements.remove(sel);
                studio.selected_element_idx = if studio.overlay.elements.is_empty() {
                    None
                } else {
                    Some(sel.min(studio.overlay.elements.len() - 1))
                };
                studio.active_color_target = None;
            }
        }
    }

    // Ctrl+D -> duplicate/clone selected element
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::D) || i.consume_key(egui::Modifiers::CTRL, egui::Key::D)) {
        if let Some(sel) = studio.selected_element_idx {
            if sel < studio.overlay.elements.len() {
                studio.push_undo_snapshot();
                let mut cloned = studio.overlay.elements[sel].clone();
                let (ew, eh) = get_element_size(&cloned);
                *cloned.id_mut() = format!("{}_copy", cloned.id());
                let cand_x = cloned.x() + 16.0;
                let cand_y = cloned.y() + 16.0;
                if cand_x + ew > studio.overlay.rect.width || cand_y + eh > studio.overlay.rect.height {
                    *cloned.x_mut() = 0.0;
                    *cloned.y_mut() = 0.0;
                } else {
                    *cloned.x_mut() = cand_x;
                    *cloned.y_mut() = cand_y;
                }
                studio.overlay.elements.insert(sel + 1, cloned);
                clamp_elements_to_canvas(&mut studio.overlay);
                studio.selected_element_idx = Some(sel + 1);
            }
        }
    }

    // Arrow keys -> nudge selected element
    let step = if studio.snap_to_grid || ctx.input(|i| i.modifiers.shift) { 16.0 } else { 1.0 };
    let mut nudge_x = 0.0f32;
    let mut nudge_y = 0.0f32;
    if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) { nudge_x -= step; }
    if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) { nudge_x += step; }
    if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) { nudge_y -= step; }
    if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) { nudge_y += step; }

    if nudge_x != 0.0 || nudge_y != 0.0 {
        studio.push_undo_snapshot();
        ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        if let Some(sel) = studio.selected_element_idx {
            if sel < studio.overlay.elements.len() {
                let (ew, eh) = get_element_size(&studio.overlay.elements[sel]);
                let elem = &mut studio.overlay.elements[sel];
                let cur_x = elem.x();
                let cur_y = elem.y();
                if matches!(elem, OverlayElement::Crosshair { .. }) {
                    *elem.x_mut() = clamp_crosshair_coord(cur_x + nudge_x, studio.overlay.rect.width);
                    *elem.y_mut() = clamp_crosshair_coord(cur_y + nudge_y, studio.overlay.rect.height);
                } else {
                    *elem.x_mut() = (cur_x + nudge_x).clamp(0.0, (studio.overlay.rect.width - ew).max(0.0));
                    *elem.y_mut() = (cur_y + nudge_y).clamp(0.0, (studio.overlay.rect.height - eh).max(0.0));
                }
            }
        }
    }

    // [ / ] -> Layer reordering (move layer down / up)
    if ctx.input(|i| i.key_pressed(egui::Key::OpenBracket)) {
        if let Some(sel) = studio.selected_element_idx {
            if sel > 0 && sel < studio.overlay.elements.len() {
                studio.push_undo_snapshot();
                let item = studio.overlay.elements.remove(sel);
                studio.overlay.elements.insert(sel - 1, item);
                studio.selected_element_idx = Some(sel - 1);
            }
        }
    }
    if ctx.input(|i| i.key_pressed(egui::Key::CloseBracket)) {
        if let Some(sel) = studio.selected_element_idx {
            if sel + 1 < studio.overlay.elements.len() {
                studio.push_undo_snapshot();
                let item = studio.overlay.elements.remove(sel);
                studio.overlay.elements.insert(sel + 1, item);
                studio.selected_element_idx = Some(sel + 1);
            }
        }
    }

    // Escape: deselect element or close color picker
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        if studio.active_color_target.is_some() {
            studio.active_color_target = None;
        } else if studio.selected_element_idx.is_some() {
            studio.selected_element_idx = None;
        }
    }
}

/// True while one inspector edit session (a DragValue drag, a slider or a colour picker) is
/// still in progress: a changed frame, or a frame with the pointer still held even when the
/// value did not move.
fn inspector_edit_session_continues(changed_this_frame: bool, pointer_down: bool) -> bool {
    changed_this_frame || pointer_down
}

/// Snapshot to push when this frame started an inspector edit session: the pre-edit state,
/// once per session, so Ctrl+Z after a colour or size change restores that change instead of
/// an unrelated older layout snapshot (U02-005).
fn inspector_undo_snapshot(
    session_active: bool,
    before: &OverlayDefinition,
    after: &OverlayDefinition,
) -> Option<OverlayDefinition> {
    if session_active || before == after {
        None
    } else {
        Some(before.clone())
    }
}

/// Appends one undo snapshot, bounded to 50 entries, and clears the redo branch. Consecutive
/// identical snapshots are ignored so a layout operation and the inspector session that saw it
/// cannot push the same state twice.
pub(super) fn push_overlay_undo(
    undo_stack: &mut Vec<OverlayDefinition>,
    redo_stack: &mut Vec<OverlayDefinition>,
    snapshot: &OverlayDefinition,
) {
    crate::gui::state::push_if_new(undo_stack, redo_stack, snapshot);
}

/// Renders the inspector and records one undo snapshot for the edit session it started
/// (U02-005). The comparison is scoped to the inspector so canvas drags keep their own
/// drag-start snapshot instead of adding one per dragged frame.
pub(super) fn render_inspector_with_edit_undo(
    ui: &mut Ui,
    studio: &mut OverlayStudioState,
    monitors: &[crate::models::MonitorInfo],
    rules_list: &[(String, String)],
    open_element_catalog: &mut bool,
    t: &Tokens,
    loc: &crate::i18n::Strings,
) {
    let session_id = egui::Id::unique(("overlay_studio_inspector_session", studio.overlay.id.as_str()));
    let session_active = ui.ctx().data(|d| d.get_temp::<bool>(session_id)).unwrap_or(false);
    let before = studio.overlay.clone();

    render_overlay_inspector(ui, studio, monitors, rules_list, open_element_catalog, t, loc);

    if let Some(snapshot) = inspector_undo_snapshot(session_active, &before, &studio.overlay) {
        push_overlay_undo(&mut studio.undo_stack, &mut studio.redo_stack, &snapshot);
    }
    let pointer_down = ui.ctx().input(|i| i.pointer.any_down());
    let changed = studio.overlay != before;
    ui.ctx()
        .data_mut(|d| d.insert_temp(session_id, inspector_edit_session_continues(changed, pointer_down)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::state::GuiState;
    use crate::gui::theme;
    use crate::gui::views::overlay_modal::render_overlay_modal;

    #[test]
    fn test_overlay_modal_tab_cycling() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);

        let mut state = GuiState::new();
        state.open_overlay_studio("overlay_telemetry");

        // Overlay telemetry starts with elements
        let num_elements = state.overlay_studio.as_ref().unwrap().overlay.elements.len();
        assert!(num_elements > 1);

        // Frame 1: Initial render, no key pressed
        let mut out = ctx.run_ui(Default::default(), |ctx| {
            render_overlay_modal(ctx, &mut state);
        });
        out.textures_delta.clear();

        // Frame 2: Press Tab -> Selects element 0
        let mut raw_input = egui::RawInput::default();
        raw_input.events.push(egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });

        let mut out = ctx.run_ui(raw_input, |ctx| {
            render_overlay_modal(ctx, &mut state);
        });
        out.textures_delta.clear();

        assert_eq!(
            state.overlay_studio.as_ref().unwrap().selected_element_idx,
            Some(0)
        );

        // Frame 3: Press Tab again -> Selects element 1
        let mut raw_input = egui::RawInput::default();
        raw_input.events.push(egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });

        let mut out = ctx.run_ui(raw_input, |ctx| {
            render_overlay_modal(ctx, &mut state);
        });
        out.textures_delta.clear();

        assert_eq!(
            state.overlay_studio.as_ref().unwrap().selected_element_idx,
            Some(1)
        );

        // Frame 4: Press Shift+Tab -> Selects element 0
        let mut raw_input = egui::RawInput::default();
        raw_input.events.push(egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::SHIFT,
        });

        let mut out = ctx.run_ui(raw_input, |ctx| {
            render_overlay_modal(ctx, &mut state);
        });
        out.textures_delta.clear();

        assert_eq!(
            state.overlay_studio.as_ref().unwrap().selected_element_idx,
            Some(0)
        );

        // Frame 5: Press Shift+Tab from 0 -> Wraps around to last element (num_elements - 1)
        let mut raw_input = egui::RawInput::default();
        raw_input.events.push(egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::SHIFT,
        });

        let mut out = ctx.run_ui(raw_input, |ctx| {
            render_overlay_modal(ctx, &mut state);
        });
        out.textures_delta.clear();

        assert_eq!(
            state.overlay_studio.as_ref().unwrap().selected_element_idx,
            Some(num_elements - 1)
        );
    }

    // ---------------- U02-005: one undo snapshot per inspector edit session ----------------

    #[test]
    fn test_property_edit_session_pushes_one_snapshot() {
        let mut studio = OverlayStudioState::new(OverlayDefinition::default_crosshair(), None, false, String::new());
        let before = studio.overlay.clone();

        // First changed frame of a session: one snapshot of the pre-edit state.
        studio.overlay.opacity = 0.5;
        let snapshot = inspector_undo_snapshot(false, &before, &studio.overlay).expect("first frame must snapshot");
        push_overlay_undo(&mut studio.undo_stack, &mut studio.redo_stack, &snapshot);
        assert_eq!(studio.undo_stack.len(), 1);

        // Later frames of the same drag must not pile up snapshots.
        studio.overlay.opacity = 0.4;
        assert!(inspector_undo_snapshot(true, &before, &studio.overlay).is_none());
        assert!(inspector_edit_session_continues(true, false));
        assert!(inspector_edit_session_continues(false, true));
        assert!(!inspector_edit_session_continues(false, false));

        // An unchanged frame never snapshots, and a duplicate snapshot is ignored.
        assert!(inspector_undo_snapshot(false, &before, &before).is_none());
        push_overlay_undo(&mut studio.undo_stack, &mut studio.redo_stack, &snapshot);
        assert_eq!(studio.undo_stack.len(), 1);

        // Ctrl+Z now restores the state from before the colour/size edit.
        studio.undo();
        assert_eq!(studio.overlay, before);
    }
}
