use egui::{pos2, vec2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use crate::gui::components::dnd;
use crate::gui::components::modal::{render_modal_header, Modal, ModalHeader};
use crate::gui::components::text::fill;
use crate::gui::components::widgets::{self, Glyph, Kind};
use crate::gui::state::{ColorPickerTarget, GuiState, OverlayStudioState};
use crate::gui::theme::{self, Tokens};
use crate::models::{
    compute_overlay_screen_pos, compute_overlay_screen_pos_with_anchor, CrosshairStyle,
    OverlayAnchor, OverlayClickAction, OverlayDefinition, OverlayElement, ShapeKind,
};

pub use crate::gui::components::overlay_renderer::get_element_size;

fn rotate_vec(v: egui::Vec2, angle_rad: f32) -> egui::Vec2 {
    let cos = angle_rad.cos();
    let sin = angle_rad.sin();
    vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
}

fn rotate_point(p: egui::Pos2, center: egui::Pos2, angle_rad: f32) -> egui::Pos2 {
    let offset = p - center;
    center + rotate_vec(offset, angle_rad)
}

fn get_rotated_aabb(center: egui::Pos2, half_w: f32, half_h: f32, angle_rad: f32) -> egui::Rect {
    let cos = angle_rad.cos().abs();
    let sin = angle_rad.sin().abs();
    let bounding_half_w = half_w * cos + half_h * sin;
    let bounding_half_h = half_w * sin + half_h * cos;
    Rect::from_center_size(center, vec2(bounding_half_w * 2.0, bounding_half_h * 2.0))
}

#[cfg(windows)]
static OVERLAY_STUDIO_LAST_Z: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(windows)]
static OVERLAY_STUDIO_LAST_Y: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);


/// Crosshair `x`/`y` use `0` — and anything negative — as the sentinel for "centre of the
/// card": the renderer and the canvas hit-test both branch on `> 0.0`. Every editing path must
/// therefore stay at 1 px or more, or the crosshair teleports to the card centre (U02-004).
const CROSSHAIR_MIN_COORD: f32 = 1.0;

/// Bounds an edited crosshair coordinate may take on one axis.
fn crosshair_coord_range(dimension: f32) -> std::ops::RangeInclusive<f32> {
    CROSSHAIR_MIN_COORD..=dimension.max(CROSSHAIR_MIN_COORD)
}

/// Clamps one crosshair coordinate into `crosshair_coord_range`.
fn clamp_crosshair_coord(value: f32, dimension: f32) -> f32 {
    let range = crosshair_coord_range(dimension);
    value.clamp(*range.start(), *range.end())
}

/// Mirrors `get_element_size` for Text so the clamp and the canvas hit-test agree on the box.
fn text_size_estimate(text: &str, font_size: f32) -> (f32, f32) {
    let char_len = (text.chars().count() as f32).max(1.0);
    let est_w = (char_len * (font_size * 0.6).max(8.0) + 12.0).max(30.0);
    let est_h = (font_size + 6.0).max(18.0);
    (est_w, est_h)
}

/// Card/element geometry that [`clamp_elements_to_canvas`] actually operates on.
///
/// The clamp rewrites element sizes and positions, and the studio persists those, so it must
/// run only when this geometry changed. Colours, labels and font sizes are deliberately not
/// part of the signature: the old unconditional per-frame call used to shrink a long Text
/// label's font permanently (U02-003).
#[derive(Clone, Debug, PartialEq)]
struct OverlayGeometry {
    card_w: f32,
    card_h: f32,
    card_x: f32,
    card_y: f32,
    elements: Vec<[f32; 4]>,
}

fn overlay_geometry(overlay: &OverlayDefinition) -> OverlayGeometry {
    OverlayGeometry {
        card_w: overlay.rect.width,
        card_h: overlay.rect.height,
        card_x: overlay.rect.x,
        card_y: overlay.rect.y,
        elements: overlay
            .elements
            .iter()
            .map(|elem| {
                let (w, h) = get_element_size(elem);
                [elem.x(), elem.y(), w, h]
            })
            .collect(),
    }
}

pub fn clamp_elements_to_canvas(overlay: &mut OverlayDefinition) {
    let canvas_w = overlay.rect.width.max(20.0);
    let canvas_h = overlay.rect.height.max(20.0);

    for elem in &mut overlay.elements {
        match elem {
            OverlayElement::Image { width, height, lock_aspect, x, y, .. } => {
                if *lock_aspect {
                    let w = *width;
                    let h = (*height).max(1.0);
                    if w > canvas_w || h > canvas_h {
                        let scale = (canvas_w / w).min(canvas_h / h);
                        *width = (w * scale).round().clamp(8.0, canvas_w);
                        *height = (h * scale).round().clamp(8.0, canvas_h);
                    } else {
                        *width = w.clamp(8.0, canvas_w);
                        *height = h.clamp(8.0, canvas_h);
                    }
                } else {
                    *width = width.clamp(8.0, canvas_w);
                    *height = height.clamp(8.0, canvas_h);
                }
                let max_x = (canvas_w - *width).max(0.0);
                let max_y = (canvas_h - *height).max(0.0);
                *x = x.clamp(0.0, max_x);
                *y = y.clamp(0.0, max_y);
            }
            OverlayElement::Shape { width, height, lock_aspect, x, y, .. } => {
                if *lock_aspect {
                    let w = *width;
                    let h = (*height).max(1.0);
                    if w > canvas_w || h > canvas_h {
                        let scale = (canvas_w / w).min(canvas_h / h);
                        *width = (w * scale).round().clamp(4.0, canvas_w);
                        *height = (h * scale).round().clamp(4.0, canvas_h);
                    } else {
                        *width = w.clamp(4.0, canvas_w);
                        *height = h.clamp(4.0, canvas_h);
                    }
                } else {
                    *width = width.clamp(4.0, canvas_w);
                    *height = height.clamp(4.0, canvas_h);
                }
                let max_x = (canvas_w - *width).max(0.0);
                let max_y = (canvas_h - *height).max(0.0);
                *x = x.clamp(0.0, max_x);
                *y = y.clamp(0.0, max_y);
            }
            OverlayElement::Crosshair { size, gap, x, y, .. } => {
                let max_dim = canvas_w.min(canvas_h);
                if (*size + *gap) * 2.0 > max_dim {
                    *size = ((max_dim / 2.0) - *gap).clamp(2.0, 128.0);
                }
                *x = clamp_crosshair_coord(*x, canvas_w);
                *y = clamp_crosshair_coord(*y, canvas_h);
            }
            OverlayElement::Text { font_size, text, x, y, .. } => {
                // `font_size` is a user choice and is never rewritten here: the old arm only
                // lowered it, so a long label shrank permanently, deleting characters never
                // restored it, and the shrunken value was persisted on close (U02-003).
                // Overflowing text is clipped by the card painter instead.
                let (est_w, est_h) = text_size_estimate(text, *font_size);
                let max_x = (canvas_w - est_w).max(0.0);
                let max_y = (canvas_h - est_h).max(0.0);
                *x = x.clamp(0.0, max_x);
                *y = y.clamp(0.0, max_y);
            }
            OverlayElement::Graph { width, height, x, y, .. }
            | OverlayElement::ClockTimer { width, height, x, y, .. }
            | OverlayElement::InteractiveControl { width, height, x, y, .. }
            | OverlayElement::SystemTelemetry { width, height, x, y, .. }
            | OverlayElement::MediaWidget { width, height, x, y, .. }
            | OverlayElement::KeyVisualizer { width, height, x, y, .. } => {
                *width = width.clamp(10.0, canvas_w);
                *height = height.clamp(4.0, canvas_h);
                let max_x = (canvas_w - *width).max(0.0);
                let max_y = (canvas_h - *height).max(0.0);
                *x = x.clamp(0.0, max_x);
                *y = y.clamp(0.0, max_y);
            }
        }
    }
}

/// Studio preview interactivity. The canvas paints the real overlay elements so the preview
/// matches what the daemon renders, but it must never forward live pointer input to the
/// renderer: `render_element` executes `on_click` (rules, `taskkill`, media keys) and mutates
/// element state on click, so a click in the editor would hit the live system while every
/// renderer-side state change dies with the preview (U02-002). Authoring-time widget testing
/// is the explicit [`studio_test_click`] path instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StudioPreviewInputs {
    is_interactive: bool,
    pointer_clicked: bool,
    pointer_down: bool,
}

fn studio_preview_render_inputs() -> StudioPreviewInputs {
    StudioPreviewInputs {
        is_interactive: false,
        pointer_clicked: false,
        pointer_down: false,
    }
}

/// Studio-local simulation of a preview click: flips `ToggleSwitch` state and starts/stops a
/// stopwatch or countdown. This is the only click path in the studio, and it edits the real
/// element instead of a throwaway clone.
fn studio_test_click(elem: &mut OverlayElement) {
    match elem {
        OverlayElement::InteractiveControl {
            control: crate::models::InteractiveControlKind::ToggleSwitch { is_active, .. },
            ..
        } => {
            *is_active = !*is_active;
        }
        OverlayElement::ClockTimer {
            is_running,
            mode,
            countdown_remaining_ms,
            countdown_total_ms,
            ..
        } if *mode == crate::models::TimerMode::Stopwatch || *mode == crate::models::TimerMode::Countdown => {
            if *mode == crate::models::TimerMode::Countdown && *countdown_remaining_ms == 0 {
                *countdown_remaining_ms = *countdown_total_ms;
            }
            *is_running = !*is_running;
        }
        _ => {}
    }
}

/// Runs [`clamp_elements_to_canvas`] only when the card/element geometry it rewrites actually
/// changed. The old unconditional per-frame call let a long Text label shrink its own font
/// permanently (U02-003); the signature is cached per overlay id in egui memory.
fn clamp_studio_geometry_if_changed(ctx: &egui::Context, overlay: &mut OverlayDefinition) {
    let geometry_id = egui::Id::unique(("overlay_studio_geometry", overlay.id.as_str()));
    let cached: Option<OverlayGeometry> = ctx.data(|d| d.get_temp(geometry_id));
    if cached.as_ref() == Some(&overlay_geometry(overlay)) {
        return;
    }
    clamp_elements_to_canvas(overlay);
    ctx.data_mut(|d| d.insert_temp(geometry_id, overlay_geometry(overlay)));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StudioHistoryAction {
    Undo,
    Redo,
}

/// Ctrl+Z / Ctrl+Y history input for the studio. Uses physical Win32 `GetAsyncKeyState` edge
/// transitions (immune to layout like Russian Cyrillic) with an egui fallback, and consumes
/// the keys so the rest of the UI never sees them.
fn read_studio_history_shortcuts(
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
fn handle_studio_element_shortcuts(
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

/// Synchronizes live on-screen calibration coordinates from the daemon. Polls only while the
/// pointer is idle so an in-progress drag is never overwritten, and never blocks the frame: the
/// request is queued and the answer is applied on a later frame. Returns `true` when the overlay
/// position changed.
fn sync_calibration_position(
    ctx: &egui::Context,
    state: &mut GuiState,
    studio: &mut OverlayStudioState,
    active_mon: &crate::models::MonitorInfo,
) -> bool {
    if !studio.is_calibrating || ctx.input(|i| i.pointer.any_down()) {
        return false;
    }
    state.request_overlay_position(&studio.overlay.id);
    let Some((abs_x, abs_y)) = state.overlay_position_for(&studio.overlay.id) else {
        return false;
    };

    let rel_x = (abs_x - active_mon.x as f32).clamp(0.0, (active_mon.width as f32 - studio.overlay.rect.width).max(0.0));
    let rel_y = (abs_y - active_mon.y as f32).clamp(0.0, (active_mon.height as f32 - studio.overlay.rect.height).max(0.0));
    let pos_differs_from_current = (studio.overlay.rect.x - rel_x).abs() > 0.5 || (studio.overlay.rect.y - rel_y).abs() > 0.5;
    let pos_differs_from_last_synced = studio.last_synced_pos.is_none_or(|(lx, ly)| (lx - abs_x).abs() > 0.5 || (ly - abs_y).abs() > 0.5);
    if !(pos_differs_from_current && pos_differs_from_last_synced) {
        return false;
    }

    studio.overlay.anchor = OverlayAnchor::Custom;
    studio.overlay.rect.x = rel_x;
    studio.overlay.rect.y = rel_y;
    studio.last_synced_pos = Some((abs_x, abs_y));
    ctx.request_repaint();
    true
}

/// Shows or hides the live calibration window that mirrors the studio card on screen. The
/// preview requests are fire-and-forget: the studio's own copy renders immediately.
fn apply_calibration_toggle(
    state: &mut GuiState,
    studio: &mut OverlayStudioState,
    active_mon: &crate::models::MonitorInfo,
) {
    studio.is_calibrating = !studio.is_calibrating;
    if studio.is_calibrating {
        let (csx, csy) = compute_overlay_screen_pos(&studio.overlay, active_mon.width as f32, active_mon.height as f32);
        let abs_x = active_mon.x as f32 + csx;
        let abs_y = active_mon.y as f32 + csy;
        studio.last_synced_pos = Some((abs_x, abs_y));
        state.request_show_overlay_preview(studio.overlay.clone(), true);
    } else {
        studio.last_synced_pos = None;
        state.request_hide_overlay_preview(&studio.overlay.id);
    }
}

/// Re-sends the edited overlay to the calibration window after a visible change.
fn push_calibration_preview(
    state: &mut GuiState,
    studio: &mut OverlayStudioState,
    initial_overlay: &OverlayDefinition,
    active_mon: &crate::models::MonitorInfo,
) {
    if !studio.is_calibrating || &studio.overlay == initial_overlay {
        return;
    }
    let (csx, csy) = compute_overlay_screen_pos(&studio.overlay, active_mon.width as f32, active_mon.height as f32);
    let abs_x = active_mon.x as f32 + csx;
    let abs_y = active_mon.y as f32 + csy;
    studio.last_synced_pos = Some((abs_x, abs_y));
    state.request_show_overlay_preview(studio.overlay.clone(), true);
}

/// Real-time auto-save of the studio overlay into the rule action being edited.
fn persist_studio_into_rule(state: &mut GuiState, studio: &OverlayStudioState) {
    let Some(target_idx) = studio.action_target_idx else { return };
    let Some(rule) = state.editing_rule.as_mut() else { return };
    let actions = if studio.is_exit_pipeline {
        &mut rule.on_exit_actions
    } else {
        &mut rule.actions
    };
    if let Some(crate::models::Action::Overlay { custom_overlay, overlay_id, .. }) = actions.get_mut(target_idx) {
        if custom_overlay.is_some() {
            *custom_overlay = Some(Box::new(studio.overlay.clone()));
        }
        *overlay_id = studio.overlay.id.clone();
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
fn push_overlay_undo(
    undo_stack: &mut Vec<OverlayDefinition>,
    redo_stack: &mut Vec<OverlayDefinition>,
    snapshot: &OverlayDefinition,
) {
    crate::gui::state::push_if_new(undo_stack, redo_stack, snapshot);
}

/// Renders the inspector and records one undo snapshot for the edit session it started
/// (U02-005). The comparison is scoped to the inspector so canvas drags keep their own
/// drag-start snapshot instead of adding one per dragged frame.
fn render_inspector_with_edit_undo(
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

pub fn render_overlay_modal(ctx: &egui::Context, state: &mut GuiState) {
    let is_open = state.overlay_studio.is_some();
    let active_studio = state
        .overlay_studio
        .as_ref()
        .or(state.last_overlay_studio.as_ref())
        .cloned();

    let Some(mut studio) = active_studio else { return };
    // Continually request repaint for live telemetry, clock, and graph animations
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    clamp_studio_geometry_if_changed(ctx, &mut studio.overlay);
    let t = Tokens::get(ctx);
    let loc = state.loc();
    let mut initial_overlay = studio.overlay.clone();

    let mut toggle_calibration = false;

    let monitors = crate::daemon::overlay_manager::get_system_monitors();
    let active_mon = resolve_overlay_monitor(&studio.overlay, &monitors);

    if sync_calibration_position(ctx, state, &mut studio, &active_mon) {
        initial_overlay = studio.overlay.clone();
    }

    let rules_list: Vec<(String, String)> = state
        .daemon_state
        .as_ref()
        .map(|s| s.config.rules.iter().map(|r| (r.id.clone(), r.name.clone())).collect())
        .unwrap_or_default();

    let mut open_element_catalog = false;

    // Studio keyboard shortcuts are processed before UI rendering so a key is consumed in the
    // frame it is pressed.
    let is_typing = ctx.text_edit_focused();
    if let Some(action) = read_studio_history_shortcuts(ctx, is_open, is_typing) {
        match action {
            StudioHistoryAction::Undo => studio.undo(),
            StudioHistoryAction::Redo => studio.redo(),
        }
        ctx.request_repaint();
    }
    handle_studio_element_shortcuts(ctx, &mut studio, is_open, is_typing);

    let modal_res = Modal::new("overlay-studio-modal", is_open)
        .width_fraction(0.92)
        .height_fraction(0.92)
        .min_size(vec2(920.0, 600.0))
        .max_size(vec2(1360.0, 940.0))
        .header_height(50.0)
        .close_on_escape(studio.active_color_target.is_none())
        .show(ctx, &t, |frame| {
            // ---------------- 1. HEADER ----------------
            frame.header(|ui, close_req| {
                render_modal_header(
                    ui,
                    close_req,
                    &t,
                    ModalHeader {
                        glyph: Some(egui_phosphor::regular::FRAME_CORNERS),
                        glyph_size: 16.0,
                        close_tip: loc.modal_tip_close,
                        left_pad: 18.0,
                        glyph_pad: 6.0,
                        right_pad: 16.0,
                        item_spacing_x: None,
                    },
                    |ui| { widgets::hud(ui, loc.overlay_studio_title, t.ink); },
                    |ui| {
                        // Calibrate Position on Screen Button
                        ui.add_space(8.0);
                        let cal_text = if studio.is_calibrating {
                            loc.overlay_studio_btn_calibrate_hide
                        } else {
                            loc.overlay_studio_btn_calibrate
                        };
                        let cal_kind = if studio.is_calibrating { Kind::Danger } else { Kind::Secondary };
                        if widgets::button(ui, cal_text, cal_kind, 0.0).clicked() {
                            toggle_calibration = true;
                        }

                        // Presets Button
                        ui.add_space(8.0);
                        let presets_kind = if studio.show_presets_popover { Kind::Primary } else { Kind::Secondary };
                        if widgets::button(ui, loc.overlay_studio_presets_btn, presets_kind, 0.0).clicked() {
                            studio.show_presets_popover = !studio.show_presets_popover;
                            if studio.show_presets_popover && studio.preset_save_name.trim().is_empty() {
                                studio.preset_save_name = studio.overlay.name.clone();
                            }
                        }
                    },
                );
            });

            // ---------------- 2. BODY: SPLIT LAYOUT ----------------
            frame.body(|ui| {
                let body_rect = ui.max_rect();
                let left_w = (body_rect.width() * 0.44).clamp(380.0, 520.0);
                let left_rect = Rect::from_min_size(body_rect.min, vec2(left_w, body_rect.height()));
                let right_rect = Rect::from_min_max(
                    pos2(left_rect.right() + 1.0, body_rect.min.y),
                    body_rect.max,
                );

                // Vertical Divider between panels
                let v_divider = Rect::from_min_max(
                    pos2(left_rect.right(), body_rect.top() + 1.0),
                    pos2(left_rect.right() + 1.0, body_rect.bottom() - 1.0),
                );
                ui.painter().rect_filled(v_divider, CornerRadius::ZERO, t.hairline().color);

                // LEFT PANEL: Overlay Settings & Elements Inspector
                ui.scope_builder(egui::UiBuilder::new().max_rect(left_rect), |ui| {
                    ui.set_width(left_rect.width());
                    ui.set_height(left_rect.height());
                    ui.set_clip_rect(left_rect);

                    egui::Frame::new()
                        .inner_margin(egui::Margin::same(14))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("overlay_studio_left_scroll")
                                .auto_shrink([false, false])
                                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                                .show(ui, |ui| {
                                    render_inspector_with_edit_undo(
                                        ui,
                                        &mut studio,
                                        &monitors,
                                        &rules_list,
                                        &mut open_element_catalog,
                                        &t,
                                        loc,
                                    );
                                });
                        });
                });

                // RIGHT PANEL: Canvas Preview
                ui.scope_builder(egui::UiBuilder::new().max_rect(right_rect), |ui| {
                    ui.set_width(right_rect.width());
                    ui.set_height(right_rect.height());
                    ui.set_clip_rect(right_rect);

                    egui::Frame::new()
                        .inner_margin(egui::Margin::same(14))
                        .show(ui, |ui| {
                            render_overlay_canvas(
                                ui,
                                &mut studio,
                                &t,
                                loc,
                            );
                        });
                });
            });
        });

    if modal_res.is_some() {
        if modal_res.as_ref().is_some_and(|r| r.close_requested) {
            state.close_overlay_studio();
        } else if is_open {
            if toggle_calibration {
                apply_calibration_toggle(state, &mut studio, &active_mon);
            }

            push_calibration_preview(state, &mut studio, &initial_overlay, &active_mon);

            persist_studio_into_rule(state, &studio);

            state.overlay_studio = Some(studio);
        }
    } else if !is_open {
        state.last_overlay_studio = None;
    }

    if open_element_catalog {
        state.open_element_catalog();
    }
}

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
                    // 1. Save Current as Preset Section (custom name input + save button)
                    widgets::hud(ui, loc.presets_save_btn, t.muted);
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let btn_w = widgets::button_width(ui, loc.presets_save_btn, 0.0);
                        let text_w = (ui.available_width() - btn_w - ui.spacing().item_spacing.x).max(100.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut studio.preset_save_name)
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
                        egui::TextEdit::singleline(&mut studio.preset_search_query)
                            .hint_text(loc.presets_search_placeholder)
                            .desired_width(ui.available_width())
                            .min_size(vec2(0.0, theme::CONTROL_H - 4.0))
                    );
                    ui.add_space(10.0);

                    // 3. Presets List
                    let all_presets = state.daemon_state.as_ref()
                        .map(|s| s.config.overlays.clone())
                        .unwrap_or_else(|| crate::config::Library::read().overlays);

                    let q = studio.preset_search_query.trim().to_lowercase();
                    let filtered: Vec<_> = all_presets.into_iter()
                        .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q) || p.id.to_lowercase().contains(&q))
                        .collect();

                    if filtered.is_empty() {
                        ui.add_space(30.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(loc.presets_empty).color(t.muted).size(13.0));
                        });
                    } else {
                        egui::ScrollArea::vertical()
                            .id_salt("overlay_presets_scroll")
                            .max_height(340.0)
                            .show(ui, |ui| {
                                let mut to_load = None;
                                let mut to_delete = None;

                                for p in &filtered {
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
                                                        ui.label(RichText::new(&p.name).font(FontId::new(13.0, theme::semibold())).color(t.ink_strong));
                                                        ui.label(
                                                            RichText::new(format!("{} {}, {}×{}", p.elements.len(), loc.presets_badge_elements, p.rect.width as i32, p.rect.height as i32))
                                                                .size(11.0)
                                                                .color(t.muted)
                                                        );
                                                    });

                                                    // 2. Action buttons on the right (second)
                                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                        if widgets::compact_button(ui, loc.presets_delete_btn, Kind::Danger, 24.0).clicked() {
                                                            to_delete = Some(p.id.clone());
                                                        }
                                                        ui.add_space(4.0);
                                                        if widgets::compact_button(ui, loc.presets_load_btn, Kind::Secondary, 24.0).clicked() {
                                                            to_load = Some(p.clone());
                                                        }
                                                    });
                                                });
                                            });
                                        });
                                    });
                                    ui.add_space(4.0);
                                }

                                if let Some(loaded) = to_load {
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

                                if let Some(del_id) = to_delete {
                                    delete_requested = Some(del_id);
                                }
                            });
                    }
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

/// Normalises an inspector-entered rotation to the same `0..360` range the canvas gestures use,
/// so one element cannot be stored as -90° or 270° depending on the edit path (U02-015).
fn normalize_rotation_deg(degrees: f32) -> f32 {
    degrees.rem_euclid(360.0)
}

fn render_rotation_control(
    ui: &mut Ui,
    rotation: &mut f32,
    t: &Tokens,
    loc: &crate::i18n::Strings,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(loc.overlay_studio_rotation).family(theme::semibold()).size(10.5).color(t.faint));
        let resp = ui.add(
            egui::DragValue::new(rotation)
                .range(0.0..=360.0)
                .speed(1.0)
                .suffix("°")
        );
        if resp.changed() {
            *rotation = normalize_rotation_deg(*rotation);
        }
    });
}

fn render_color_chip(
    ui: &mut Ui,
    label: &str,
    color: &mut [u8; 4],
    target: ColorPickerTarget,
    active_target: &mut Option<ColorPickerTarget>,
    t: &Tokens,
) {
    let is_active = *active_target == Some(target);
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).family(theme::semibold()).size(10.5).color(t.faint));

        // Compact rounded rectangle swatch (vec2(28.0, 15.0)), no large square, no hex text
        let swatch_size = vec2(28.0, 15.0);
        let id = ui.scope_id().with(("swatch", target, label));
        let (rect, _) = ui.allocate_exact_size(swatch_size, Sense::hover());
        let resp = ui.interact(rect, id, Sense::click());
        let p = ui.painter();

        // 1. Checkerboard pattern for alpha transparency
        let half_w = rect.width() / 2.0;
        let half_h = rect.height() / 2.0;
        p.rect_filled(rect, CornerRadius::same(3), Color32::from_gray(50));
        p.rect_filled(Rect::from_min_size(rect.min, vec2(half_w, half_h)), CornerRadius::ZERO, Color32::from_gray(90));
        p.rect_filled(Rect::from_min_size(rect.min + vec2(half_w, half_h), vec2(half_w, half_h)), CornerRadius::ZERO, Color32::from_gray(90));

        // 2. Color fill with alpha
        let col = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
        p.rect_filled(rect, CornerRadius::same(3), col);

        // 3. Border stroke: accent ink if active, subtle hairline otherwise
        let stroke = if is_active {
            Stroke::new(1.5, t.accent_ink)
        } else if resp.hovered() {
            Stroke::new(1.0, Color32::WHITE)
        } else {
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 30))
        };
        p.rect_stroke(rect, CornerRadius::same(3), stroke, egui::StrokeKind::Inside);

        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            if is_active {
                *active_target = None;
            } else {
                *active_target = Some(target);
            }
        }
    });
}

fn render_inline_color_editor(
    ui: &mut Ui,
    color: &mut [u8; 4],
    has_alpha: bool,
    t: &Tokens,
    loc: &crate::i18n::Strings,
) {
    egui::Frame::new()
        .fill(t.sunken)
        .stroke(Stroke::new(1.0, t.accent_ink))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;

            // Maintain stable HSVA state across frames to prevent hue/saturation snapping at extremes
            let hsva_id = ui.scope_id().with("custom_picker_hsva");
            let last_rgba_id = ui.scope_id().with("custom_picker_last_rgba");

            let last_rgba: [u8; 4] = ui.ctx().data(|d| d.get_temp(last_rgba_id)).unwrap_or([255, 255, 255, 255]);
            let mut hsva: egui::ecolor::Hsva = ui.ctx().data(|d| d.get_temp(hsva_id)).unwrap_or_else(|| {
                egui::ecolor::Hsva::from_srgba_unmultiplied(*color)
            });

            if *color != last_rgba {
                hsva = egui::ecolor::Hsva::from_srgba_unmultiplied(*color);
            }

            // Row 1: Quick Presets Palette + Direct HEX input in single row
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let presets: [(&str, [u8; 4]); 7] = [
                    (loc.overlay_studio_palette_cyan, [0, 212, 255, 255]),
                    (loc.overlay_studio_palette_green, [34, 197, 94, 255]),
                    (loc.overlay_studio_palette_red, [239, 68, 68, 255]),
                    (loc.overlay_studio_palette_amber, [245, 158, 11, 255]),
                    (loc.overlay_studio_palette_white, [255, 255, 255, 255]),
                    (loc.overlay_studio_palette_dark, [24, 28, 38, 220]),
                    (loc.overlay_studio_palette_clear, [0, 0, 0, 0]),
                ];
                for (name, col) in presets {
                    let (rect, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                    let c32 = Color32::from_rgba_unmultiplied(col[0], col[1], col[2], col[3]);
                    ui.painter().rect_filled(rect, CornerRadius::same(2), c32);
                    ui.painter().rect_stroke(rect, CornerRadius::same(2), t.hairline(), egui::StrokeKind::Inside);
                    if resp.on_hover_text(name).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        color[0] = col[0];
                        color[1] = col[1];
                        color[2] = col[2];
                        if has_alpha {
                            color[3] = col[3];
                        }
                        hsva = egui::ecolor::Hsva::from_srgba_unmultiplied(*color);
                    }
                }

                ui.add_space(2.0);

                let hex_id = ui.scope_id().with("hex_edit_buf");
                let mut hex_val: String = ui.ctx().data(|d| d.get_temp(hex_id)).unwrap_or_else(|| {
                    if has_alpha {
                        format!("#{:02X}{:02X}{:02X}{:02X}", color[0], color[1], color[2], color[3])
                    } else {
                        format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2])
                    }
                });
                let prev_hex = hex_val.clone();
                let txt_resp = ui.add(
                    egui::TextEdit::singleline(&mut hex_val)
                        .desired_width(ui.available_width())
                        .font(FontId::new(11.0, theme::mono()))
                );
                if txt_resp.changed() {
                    let clean = hex_val.trim().trim_start_matches('#');
                    if clean.len() == 6 {
                        if let Ok(v) = u32::from_str_radix(clean, 16) {
                            color[0] = ((v >> 16) & 0xFF) as u8;
                            color[1] = ((v >> 8) & 0xFF) as u8;
                            color[2] = (v & 0xFF) as u8;
                            hsva = egui::ecolor::Hsva::from_srgba_unmultiplied(*color);
                        }
                    } else if clean.len() == 8 {
                        if let Ok(v) = u32::from_str_radix(clean, 16) {
                            color[0] = ((v >> 24) & 0xFF) as u8;
                            color[1] = ((v >> 16) & 0xFF) as u8;
                            color[2] = ((v >> 8) & 0xFF) as u8;
                            color[3] = (v & 0xFF) as u8;
                            hsva = egui::ecolor::Hsva::from_srgba_unmultiplied(*color);
                        }
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(hex_id, hex_val.clone()));
                } else if !txt_resp.has_focus() {
                    let latest_hex = if has_alpha {
                        format!("#{:02X}{:02X}{:02X}{:02X}", color[0], color[1], color[2], color[3])
                    } else {
                        format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2])
                    };
                    if latest_hex != prev_hex {
                        ui.ctx().data_mut(|d| d.insert_temp(hex_id, latest_hex));
                    }
                }
            });

            // Row 2: Full-width 2D Saturation / Value gradient canvas
            let full_w = ui.available_width();
            let sv_h = 110.0;
            let (sv_rect, sv_resp) = ui.allocate_exact_size(vec2(full_w, sv_h), Sense::click_and_drag());

            let pure_hue = Color32::from(egui::ecolor::Hsva::new(hsva.h, 1.0, 1.0, 1.0));
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(pos2(sv_rect.left(), sv_rect.top()), Color32::WHITE);
            mesh.colored_vertex(pos2(sv_rect.right(), sv_rect.top()), pure_hue);
            mesh.colored_vertex(pos2(sv_rect.left(), sv_rect.bottom()), Color32::BLACK);
            mesh.colored_vertex(pos2(sv_rect.right(), sv_rect.bottom()), Color32::BLACK);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(2, 1, 3);
            ui.painter().add(egui::Shape::mesh(mesh));
            ui.painter().rect_stroke(sv_rect, CornerRadius::same(3), t.hairline(), egui::StrokeKind::Inside);

            let pointer_pos = ui.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()));
            if sv_resp.clicked() || sv_resp.dragged() {
                if let Some(pos) = pointer_pos {
                    hsva.s = ((pos.x - sv_rect.left()) / sv_rect.width()).clamp(0.0, 1.0);
                    hsva.v = (1.0 - (pos.y - sv_rect.top()) / sv_rect.height()).clamp(0.0, 1.0);
                }
            }

            // Crosshair indicator on current (s, v)
            let cx = sv_rect.left() + hsva.s * sv_rect.width();
            let cy = sv_rect.top() + (1.0 - hsva.v) * sv_rect.height();
            ui.painter().circle(pos2(cx, cy), 4.5, Color32::TRANSPARENT, Stroke::new(1.8, Color32::BLACK));
            ui.painter().circle(pos2(cx, cy), 3.5, Color32::TRANSPARENT, Stroke::new(1.5, Color32::WHITE));

            // Row 3: Full-width Hue spectrum bar
            let (hue_rect, hue_resp) = ui.allocate_exact_size(vec2(full_w, 13.0), Sense::click_and_drag());
            let stops: [(f32, Color32); 7] = [
                (0.0, Color32::from_rgb(255, 0, 0)),
                (0.1666, Color32::from_rgb(255, 255, 0)),
                (0.3333, Color32::from_rgb(0, 255, 0)),
                (0.5, Color32::from_rgb(0, 255, 255)),
                (0.6666, Color32::from_rgb(0, 0, 255)),
                (0.8333, Color32::from_rgb(255, 0, 255)),
                (1.0, Color32::from_rgb(255, 0, 0)),
            ];
            let mut h_mesh = egui::Mesh::default();
            for i in 0..6 {
                let (p0, c0) = stops[i];
                let (p1, c1) = stops[i + 1];
                let x0 = hue_rect.left() + p0 * hue_rect.width();
                let x1 = hue_rect.left() + p1 * hue_rect.width();
                let v_start = h_mesh.vertices.len() as u32;
                h_mesh.colored_vertex(pos2(x0, hue_rect.top()), c0);
                h_mesh.colored_vertex(pos2(x1, hue_rect.top()), c1);
                h_mesh.colored_vertex(pos2(x0, hue_rect.bottom()), c0);
                h_mesh.colored_vertex(pos2(x1, hue_rect.bottom()), c1);
                h_mesh.add_triangle(v_start, v_start + 1, v_start + 2);
                h_mesh.add_triangle(v_start + 2, v_start + 1, v_start + 3);
            }
            ui.painter().add(egui::Shape::mesh(h_mesh));
            ui.painter().rect_stroke(hue_rect, CornerRadius::same(2), t.hairline(), egui::StrokeKind::Inside);

            if hue_resp.clicked() || hue_resp.dragged() {
                if let Some(pos) = pointer_pos {
                    hsva.h = ((pos.x - hue_rect.left()) / hue_rect.width()).clamp(0.0, 1.0);
                }
            }

            let h_thumb_x = hue_rect.left() + hsva.h * hue_rect.width();
            ui.painter().rect(
                Rect::from_center_size(pos2(h_thumb_x, hue_rect.center().y), vec2(5.0, 15.0)),
                CornerRadius::same(2),
                Color32::WHITE,
                Stroke::new(1.0, Color32::BLACK),
                egui::StrokeKind::Middle,
            );

            // Row 4: Full-width Alpha bar (if has_alpha)
            if has_alpha {
                let (alpha_rect, alpha_resp) = ui.allocate_exact_size(vec2(full_w, 13.0), Sense::click_and_drag());
                let half_sq = 4.0;
                let mut bx = alpha_rect.left();
                let mut row = 0;
                while bx < alpha_rect.right() {
                    let mut by = alpha_rect.top();
                    let mut col = 0;
                    while by < alpha_rect.bottom() {
                        let fill = if (row + col) % 2 == 0 { Color32::from_gray(60) } else { Color32::from_gray(100) };
                        let r = Rect::from_min_size(pos2(bx, by), vec2(half_sq, half_sq)).intersect(alpha_rect);
                        ui.painter().rect_filled(r, CornerRadius::ZERO, fill);
                        by += half_sq;
                        col += 1;
                    }
                    bx += half_sq;
                    row += 1;
                }

                let col_0 = Color32::from(egui::ecolor::Hsva::new(hsva.h, hsva.s, hsva.v, 0.0));
                let col_1 = Color32::from(egui::ecolor::Hsva::new(hsva.h, hsva.s, hsva.v, 1.0));
                let mut a_mesh = egui::Mesh::default();
                a_mesh.colored_vertex(pos2(alpha_rect.left(), alpha_rect.top()), col_0);
                a_mesh.colored_vertex(pos2(alpha_rect.right(), alpha_rect.top()), col_1);
                a_mesh.colored_vertex(pos2(alpha_rect.left(), alpha_rect.bottom()), col_0);
                a_mesh.colored_vertex(pos2(alpha_rect.right(), alpha_rect.bottom()), col_1);
                a_mesh.add_triangle(0, 1, 2);
                a_mesh.add_triangle(2, 1, 3);
                ui.painter().add(egui::Shape::mesh(a_mesh));
                ui.painter().rect_stroke(alpha_rect, CornerRadius::same(2), t.hairline(), egui::StrokeKind::Inside);

                if alpha_resp.clicked() || alpha_resp.dragged() {
                    if let Some(pos) = pointer_pos {
                        hsva.a = ((pos.x - alpha_rect.left()) / alpha_rect.width()).clamp(0.0, 1.0);
                    }
                }

                let a_thumb_x = alpha_rect.left() + hsva.a * alpha_rect.width();
                ui.painter().rect(
                    Rect::from_center_size(pos2(a_thumb_x, alpha_rect.center().y), vec2(5.0, 15.0)),
                    CornerRadius::same(2),
                    Color32::WHITE,
                    Stroke::new(1.0, Color32::BLACK),
                    egui::StrokeKind::Middle,
                );
            }

            // Sync updated HSVA back to RGBA
            let c32 = Color32::from(hsva);
            color[0] = c32.r();
            color[1] = c32.g();
            color[2] = c32.b();
            if has_alpha {
                color[3] = (hsva.a * 255.0).round() as u8;
            }

            ui.ctx().data_mut(|d| {
                d.insert_temp(hsva_id, hsva);
                d.insert_temp(last_rgba_id, *color);
            });
        });
}

#[derive(Clone, Copy, Debug, Default)]
struct ElementDndSession {
    source_idx: usize,
    target_idx: usize,
}

/// Overlay element list drop line: 20 px, 0.16 s, rail inset 20/8 and a plain 10 px bead.
const ELEM_DROP_LINE_STYLE: dnd::DropLineStyle = dnd::DropLineStyle {
    height: 20.0,
    anim_secs: 0.16,
    corner: CornerRadius::same(2),
    fill_alpha: 0.08,
    line_width: 2.0,
    line_left: 20.0,
    line_right: 8.0,
    bead: Some(dnd::DropBead {
        left: 10.0,
        radius: 3.5,
        halo_radius: None,
    }),
};

/// Element drag preview chrome: a flat card with a 1 px accent stroke.
const ELEM_PREVIEW_STYLE: dnd::FloatingPreviewStyle = dnd::FloatingPreviewStyle {
    corner: CornerRadius::same(4),
    stroke_width: 1.0,
    shadow: false,
};

fn render_elem_drop_slot(
    ui: &mut Ui,
    slot_idx: usize,
    active_dst: Option<usize>,
    active_src: Option<usize>,
    total_elems: usize,
    t: &Tokens,
) {
    let is_targeted = if let (Some(dst), Some(src)) = (active_dst, active_src) {
        if slot_idx == total_elems {
            dst == slot_idx && src + 1 < total_elems
        } else {
            dst == slot_idx && slot_idx != src && slot_idx != src + 1
        }
    } else {
        false
    };

    let anim_id = ui.scope_id().with(("elem_drop_anim", slot_idx));
    dnd::drop_line(ui, anim_id, is_targeted, &ELEM_DROP_LINE_STYLE, t);
}

// ----------------- OVERLAY INSPECTOR PANEL -----------------

/// Colour slots one element panel can edit. Card-scoped targets are deliberately absent: an
/// element colour chip that opened the card editor silently rewrote the overlay style (U02-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementColorSlot {
    Primary,
    Secondary,
    Tertiary,
}

/// Builds the element-scoped picker target for one colour slot.
fn element_color_target(elem_idx: usize, slot: ElementColorSlot) -> ColorPickerTarget {
    match slot {
        ElementColorSlot::Primary => ColorPickerTarget::ElementPrimary(elem_idx),
        ElementColorSlot::Secondary => ColorPickerTarget::ElementSecondary(elem_idx),
        ElementColorSlot::Tertiary => ColorPickerTarget::ElementTertiary(elem_idx),
    }
}

/// Which colour slot of `elem_idx` the open inline picker belongs to. `None` also covers the
/// card-scoped targets, which only the card panel renders.
fn active_element_slot(target: Option<ColorPickerTarget>, elem_idx: usize) -> Option<ElementColorSlot> {
    match target? {
        ColorPickerTarget::ElementPrimary(idx) if idx == elem_idx => Some(ElementColorSlot::Primary),
        ColorPickerTarget::ElementSecondary(idx) if idx == elem_idx => Some(ElementColorSlot::Secondary),
        ColorPickerTarget::ElementTertiary(idx) if idx == elem_idx => Some(ElementColorSlot::Tertiary),
        _ => None,
    }
}

/// Per-kind minimum element size, shared by the inspector rows and the canvas clamp so the
/// hand-maintained constant sets cannot drift apart (U02-010).
#[derive(Debug, Clone, Copy, PartialEq)]
struct SizeLimits {
    min_w: f32,
    min_h: f32,
}

const IMAGE_SIZE_LIMITS: SizeLimits = SizeLimits { min_w: 8.0, min_h: 8.0 };
const SHAPE_SIZE_LIMITS: SizeLimits = SizeLimits { min_w: 4.0, min_h: 4.0 };
const PANEL_SIZE_LIMITS: SizeLimits = SizeLimits { min_w: 20.0, min_h: 15.0 };
const MEDIA_SIZE_LIMITS: SizeLimits = SizeLimits { min_w: 40.0, min_h: 20.0 };
const KEYVISUALIZER_SIZE_LIMITS: SizeLimits = SizeLimits { min_w: 30.0, min_h: 18.0 };

/// Lock-aspect toggle button, shared by the card and the Image/Shape panels (U02-010).
fn render_lock_aspect_toggle(
    ui: &mut Ui,
    locked: bool,
    glyph_size: f32,
    tooltip_locked: &str,
    tooltip_unlocked: &str,
    t: &Tokens,
) -> bool {
    let glyph = if locked {
        egui_phosphor::regular::LOCK
    } else {
        egui_phosphor::regular::LOCK_OPEN
    };
    let color = if locked { t.accent_ink } else { t.faint };
    let tip = if locked { tooltip_locked } else { tooltip_unlocked };
    let (rect, resp) = ui.allocate_exact_size(vec2(theme::CONTROL_H, theme::CONTROL_H), Sense::click());
    if locked {
        ui.painter().rect_filled(rect, CornerRadius::same(3), t.accent_ink.gamma_multiply(0.2));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(3), t.sunken);
    }
    ui.painter().rect_stroke(rect, CornerRadius::same(3), t.hairline(), egui::StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        FontId::new(glyph_size, theme::phosphor()),
        color,
    );
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// W/H row. `lock_aspect` adds the aspect-lock toggle for the element kinds that have one; the
/// rest share the plain two-column row with their own minimum sizes (U02-010).
#[allow(clippy::too_many_arguments)]
fn render_size_rows(
    ui: &mut Ui,
    width: &mut f32,
    height: &mut f32,
    lock_aspect: Option<&mut bool>,
    canvas_w: f32,
    canvas_h: f32,
    limits: SizeLimits,
    lock_tooltips: (&str, &str),
    loc: &crate::i18n::Strings,
    t: &Tokens,
) {
    match lock_aspect {
        Some(lock_aspect) => {
            ui.horizontal(|ui| {
                let aspect = if *height > 0.0 { *width / *height } else { 1.0 };
                let total_w = ui.available_width();
                let btn_w = theme::CONTROL_H;
                let spacing = ui.spacing().item_spacing.x;
                let field_w = ((total_w - btn_w - spacing * 2.0 - 4.0) / 2.0).max(40.0).floor();
                let w_resp = ui.add_sized(
                    vec2(field_w, theme::CONTROL_H),
                    egui::DragValue::new(width)
                        .range(limits.min_w..=canvas_w.max(limits.min_w))
                        .prefix(loc.overlay_studio_field_w)
                        .suffix(loc.overlay_studio_unit_px)
                        .speed(1.0),
                );
                let h_resp = ui.add_sized(
                    vec2(field_w, theme::CONTROL_H),
                    egui::DragValue::new(height)
                        .range(limits.min_h..=canvas_h.max(limits.min_h))
                        .prefix(loc.overlay_studio_field_h)
                        .suffix(loc.overlay_studio_unit_px)
                        .speed(1.0),
                );
                if *lock_aspect {
                    if w_resp.changed() && aspect > 0.0 {
                        *height = (*width / aspect).round().clamp(limits.min_h, canvas_h);
                    } else if h_resp.changed() {
                        *width = (*height * aspect).round().clamp(limits.min_w, canvas_w);
                    }
                }
                if render_lock_aspect_toggle(ui, *lock_aspect, 13.0, lock_tooltips.0, lock_tooltips.1, t) {
                    *lock_aspect = !*lock_aspect;
                }
            });
        }
        None => {
            ui.columns(2, |cols| {
                cols[0].add(
                    egui::DragValue::new(width)
                        .range(limits.min_w..=canvas_w.max(limits.min_w))
                        .prefix(loc.overlay_studio_field_w)
                        .suffix(loc.overlay_studio_unit_px),
                );
                cols[1].add(
                    egui::DragValue::new(height)
                        .range(limits.min_h..=canvas_h.max(limits.min_h))
                        .prefix(loc.overlay_studio_field_h)
                        .suffix(loc.overlay_studio_unit_px),
                );
            });
        }
    }
}

/// X/Y row, clamped so the element stays inside the card.
#[allow(clippy::too_many_arguments)]
fn render_position_rows(ui: &mut Ui, x: &mut f32, y: &mut f32, width: f32, height: f32, canvas_w: f32, canvas_h: f32, loc: &crate::i18n::Strings) {
    ui.columns(2, |cols| {
        let max_x = (canvas_w - width).max(0.0);
        let max_y = (canvas_h - height).max(0.0);
        cols[0].add(egui::DragValue::new(x).range(0.0..=max_x).prefix(loc.overlay_studio_field_x).suffix(loc.overlay_studio_unit_px));
        cols[1].add(egui::DragValue::new(y).range(0.0..=max_y).prefix(loc.overlay_studio_field_y).suffix(loc.overlay_studio_unit_px));
    });
}

/// Everything one element panel needs besides the element itself.
struct ElementPanelCtx<'a> {
    canvas_w: f32,
    canvas_h: f32,
    click_through: bool,
    active_color_target: &'a mut Option<ColorPickerTarget>,
    rules_list: &'a [(String, String)],
    t: &'a Tokens,
    loc: &'a crate::i18n::Strings,
}

fn render_overlay_inspector(
    ui: &mut Ui,
    studio: &mut OverlayStudioState,
    monitors: &[crate::models::MonitorInfo],
    rules_list: &[(String, String)],
    open_element_catalog: &mut bool,
    t: &Tokens,
    loc: &crate::i18n::Strings,
) {
    ui.spacing_mut().item_spacing.y = 8.0;

    render_overlay_card_panel(
        ui,
        &mut studio.overlay,
        monitors,
        studio.is_calibrating,
        &mut studio.card_lock_aspect,
        &mut studio.active_color_target,
        t,
        loc,
    );

    render_overlay_element_list(
        ui,
        &mut studio.overlay,
        &mut studio.undo_stack,
        &mut studio.redo_stack,
        &mut studio.selected_element_idx,
        &mut studio.active_color_target,
        rules_list,
        open_element_catalog,
        t,
        loc,
    );
}

/// Card-level settings: anchor, screen position, card geometry, opacity and the card style.
#[allow(clippy::too_many_arguments)]
fn render_overlay_card_panel(
    ui: &mut Ui,
    overlay: &mut OverlayDefinition,
    monitors: &[crate::models::MonitorInfo],
    is_calibrating: bool,
    card_lock_aspect: &mut bool,
    active_color_target: &mut Option<ColorPickerTarget>,
    t: &Tokens,
    loc: &crate::i18n::Strings,
) {
    let active_mon = resolve_overlay_monitor(overlay, monitors);
    let screen_w = active_mon.width as f32;
    let screen_h = active_mon.height as f32;

    // Anchor Picker
    widgets::hud(ui, loc.overlay_studio_field_anchor, t.faint);
    let anchor_label = match overlay.anchor {
        OverlayAnchor::ScreenCenter => loc.overlay_studio_anchor_center,
        OverlayAnchor::TopLeft => loc.overlay_studio_anchor_top_left,
        OverlayAnchor::TopRight => loc.overlay_studio_anchor_top_right,
        OverlayAnchor::BottomLeft => loc.overlay_studio_anchor_bottom_left,
        OverlayAnchor::BottomRight => loc.overlay_studio_anchor_bottom_right,
        OverlayAnchor::Custom => loc.overlay_studio_anchor_custom,
        OverlayAnchor::WindowRelative => loc.overlay_studio_anchor_window,
    };
    let prev_anchor = overlay.anchor;
    egui::ComboBox::from_id_salt("overlay_anchor_dropdown")
        .selected_text(anchor_label)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut overlay.anchor, OverlayAnchor::ScreenCenter, loc.overlay_studio_anchor_center);
            ui.selectable_value(&mut overlay.anchor, OverlayAnchor::TopLeft, loc.overlay_studio_anchor_top_left);
            ui.selectable_value(&mut overlay.anchor, OverlayAnchor::TopRight, loc.overlay_studio_anchor_top_right);
            ui.selectable_value(&mut overlay.anchor, OverlayAnchor::BottomLeft, loc.overlay_studio_anchor_bottom_left);
            ui.selectable_value(&mut overlay.anchor, OverlayAnchor::BottomRight, loc.overlay_studio_anchor_bottom_right);
            ui.selectable_value(&mut overlay.anchor, OverlayAnchor::Custom, loc.overlay_studio_anchor_custom);
            ui.selectable_value(&mut overlay.anchor, OverlayAnchor::WindowRelative, loc.overlay_studio_anchor_window);
        });

    if overlay.anchor != prev_anchor {
        match overlay.anchor {
            OverlayAnchor::ScreenCenter => {
                overlay.rect.x = 0.0;
                overlay.rect.y = 0.0;
            }
            OverlayAnchor::TopLeft | OverlayAnchor::TopRight | OverlayAnchor::BottomLeft | OverlayAnchor::BottomRight => {
                overlay.rect.x = 16.0;
                overlay.rect.y = 16.0;
            }
            OverlayAnchor::Custom => {
                let (csx, csy) = compute_overlay_screen_pos_with_anchor(prev_anchor, overlay, screen_w, screen_h);
                overlay.rect.x = csx.clamp(0.0, (screen_w - overlay.rect.width).max(0.0));
                overlay.rect.y = csy.clamp(0.0, (screen_h - overlay.rect.height).max(0.0));
            }
            OverlayAnchor::WindowRelative => {
                overlay.rect.x = 16.0;
                overlay.rect.y = 16.0;
            }
        }
        clamp_card_to_monitor(overlay, screen_w, screen_h);
    }

    if overlay.anchor == OverlayAnchor::Custom || is_calibrating {
        let max_x = (screen_w - overlay.rect.width).max(0.0);
        let max_y = (screen_h - overlay.rect.height).max(0.0);

        let (mut cur_x, mut cur_y) = if overlay.anchor == OverlayAnchor::Custom {
            (overlay.rect.x, overlay.rect.y)
        } else {
            compute_overlay_screen_pos(overlay, screen_w, screen_h)
        };

        let mut pos_changed = false;
        ui.columns(2, |cols| {
            if cols[0].add(
                egui::DragValue::new(&mut cur_x)
                    .range(0.0..=max_x)
                    .prefix(loc.overlay_studio_field_screen_x)
                    .suffix(loc.overlay_studio_unit_px)
                    .speed(1.0),
            ).changed() {
                overlay.anchor = OverlayAnchor::Custom;
                overlay.rect.x = cur_x;
                pos_changed = true;
            }
            if cols[1].add(
                egui::DragValue::new(&mut cur_y)
                    .range(0.0..=max_y)
                    .prefix(loc.overlay_studio_field_screen_y)
                    .suffix(loc.overlay_studio_unit_px)
                    .speed(1.0),
            ).changed() {
                overlay.anchor = OverlayAnchor::Custom;
                overlay.rect.y = cur_y;
                pos_changed = true;
            }
        });
        if pos_changed {
            clamp_card_to_monitor(overlay, screen_w, screen_h);
        }
    }

    // Geometry Dimensions (W x H) with Lock Aspect Ratio button
    ui.add_space(6.0);
    widgets::hud(ui, loc.overlay_studio_card_dimensions, t.faint);

    let (max_w, max_h) = max_card_dimensions(overlay, screen_w, screen_h);

    let total_w = ui.available_width();
    let btn_w = theme::CONTROL_H;
    let spacing = ui.spacing().item_spacing.x;
    let field_w = ((total_w - btn_w - spacing * 2.0 - 4.0) / 2.0).max(40.0).floor();

    let prev_w = overlay.rect.width;
    let prev_h = overlay.rect.height;
    let aspect = if prev_h > 0.0 { prev_w / prev_h } else { 1.0 };

    let mut dim_changed = false;
    ui.horizontal(|ui| {
        let w_resp = ui.add_sized(
            vec2(field_w, theme::CONTROL_H),
            egui::DragValue::new(&mut overlay.rect.width)
                .range(20.0..=max_w)
                .prefix(loc.overlay_studio_field_width)
                .suffix(loc.overlay_studio_unit_px)
                .speed(1.0),
        );
        let h_resp = ui.add_sized(
            vec2(field_w, theme::CONTROL_H),
            egui::DragValue::new(&mut overlay.rect.height)
                .range(20.0..=max_h)
                .prefix(loc.overlay_studio_field_height)
                .suffix(loc.overlay_studio_unit_px)
                .speed(1.0),
        );

        if *card_lock_aspect {
            let effective_max_w = max_w.min(max_h * aspect);
            let effective_max_h = max_h.min(max_w / aspect.max(0.01));
            if w_resp.changed() && aspect > 0.0 {
                overlay.rect.width = overlay.rect.width.clamp(20.0, effective_max_w);
                overlay.rect.height = (overlay.rect.width / aspect).round().clamp(20.0, effective_max_h);
                dim_changed = true;
            } else if h_resp.changed() {
                overlay.rect.height = overlay.rect.height.clamp(20.0, effective_max_h);
                overlay.rect.width = (overlay.rect.height * aspect).round().clamp(20.0, effective_max_w);
                dim_changed = true;
            }
        } else {
            if w_resp.changed() {
                overlay.rect.width = overlay.rect.width.clamp(20.0, max_w);
                dim_changed = true;
            }
            if h_resp.changed() {
                overlay.rect.height = overlay.rect.height.clamp(20.0, max_h);
                dim_changed = true;
            }
        }

        let lock_tip = if *card_lock_aspect {
            loc.overlay_studio_card_lock_aspect
        } else {
            loc.overlay_studio_card_unlock_aspect
        };
        if render_lock_aspect_toggle(ui, *card_lock_aspect, 14.0, lock_tip, lock_tip, t) {
            *card_lock_aspect = !*card_lock_aspect;
        }
    });
    if dim_changed {
        clamp_elements_to_canvas(overlay);
    }

    // Global Opacity Slider
    ui.add_space(6.0);
    widgets::hud(ui, loc.overlay_studio_opacity, t.faint);
    ui.add(
        egui::Slider::new(&mut overlay.opacity, 0.05..=1.0)
            .show_value(true)
            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
    );

    // Style Card Settings: Colors in 2 columns
    ui.add_space(6.0);
    widgets::hud(ui, loc.overlay_studio_card_colors, t.faint);
    ui.columns(2, |cols| {
        render_color_chip(&mut cols[0], loc.overlay_studio_chip_bg, &mut overlay.style.bg_color, ColorPickerTarget::CardBg, &mut *active_color_target, t);
        render_color_chip(&mut cols[1], loc.overlay_studio_chip_border, &mut overlay.style.border_color, ColorPickerTarget::CardBorder, &mut *active_color_target, t);
    });
    if let Some(target) = *active_color_target {
        match target {
            ColorPickerTarget::CardBg => render_inline_color_editor(ui, &mut overlay.style.bg_color, true, t, loc),
            ColorPickerTarget::CardBorder => render_inline_color_editor(ui, &mut overlay.style.border_color, true, t, loc),
            _ => {}
        }
    }

    // Border Width & Corner Radius in 2 columns
    ui.columns(2, |cols| {
        cols[0].add(egui::DragValue::new(&mut overlay.style.border_width).range(0.0..=12.0).prefix(loc.overlay_studio_field_border).suffix(loc.overlay_studio_unit_px));
        cols[1].add(egui::DragValue::new(&mut overlay.style.corner_radius).range(0.0..=32.0).prefix(loc.overlay_studio_field_radius).suffix(loc.overlay_studio_unit_px));
    });

    widgets::hrule(ui);
}

/// The element cards: drag-reorder list, per-kind property panel, clone and remove.
#[allow(clippy::too_many_arguments)]
fn render_overlay_element_list(
    ui: &mut Ui,
    overlay: &mut OverlayDefinition,
    undo_stack: &mut Vec<OverlayDefinition>,
    redo_stack: &mut Vec<OverlayDefinition>,
    selected_idx: &mut Option<usize>,
    active_color_target: &mut Option<ColorPickerTarget>,
    rules_list: &[(String, String)],
    open_element_catalog: &mut bool,
    t: &Tokens,
    loc: &crate::i18n::Strings,
) {
    ui.add_space(6.0);
    widgets::hud(ui, loc.overlay_studio_elements_title, t.ink);

    ui.add_space(4.0);

    let dnd_id = ui.scope_id().with("overlay_element_dnd");
    let mut dnd_session: Option<ElementDndSession> = ui.ctx().data(|d| d.get_temp(dnd_id));
    let pointer_pos = ui.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()));
    let pointer_released = ui.input(|i| i.pointer.any_released());

    // Cancel on Escape
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        dnd_session = None;
        ui.ctx().data_mut(|d| d.remove_temp::<ElementDndSession>(dnd_id));
    }

    let mut remove_idx = None;
    let mut clone_idx = None;
    let mut drag_started_idx = None;
    let mut card_centers = Vec::new();

    // Edge auto-scrolling while dragging elements
    if dnd_session.is_some() {
        dnd::wheel_scroll(ui);

        if let Some(pointer) = pointer_pos {
            dnd::edge_autoscroll(ui, pointer, 70.0);
        }
    }

    let active_src = dnd_session.map(|s| s.source_idx);
    let active_dst = dnd_session.map(|s| s.target_idx);

    let canvas_w = overlay.rect.width;
    let canvas_h = overlay.rect.height;
    let click_through = overlay.click_through;

    let total_elems = overlay.elements.len();
    if total_elems == 0 {
        ui.add_space(8.0);
        if widgets::full_width_action_button(ui, loc.overlay_studio_btn_add_element_large, "", t) {
            *open_element_catalog = true;
        }
        ui.add_space(8.0);
    }
    for (idx, elem) in overlay.elements.iter_mut().enumerate() {
        // Drop slot indicator before each item
        render_elem_drop_slot(ui, idx, active_dst, active_src, total_elems, t);

        let is_selected = *selected_idx == Some(idx);
        let frame_stroke = if is_selected {
            Stroke::new(1.5, t.accent_ink)
        } else {
            t.hairline()
        };
        let frame_fill = if is_selected {
            t.sunken.gamma_multiply(1.5)
        } else {
            t.sunken
        };

        let card_resp = egui::Frame::new()
            .fill(frame_fill)
            .stroke(frame_stroke)
            .corner_radius(CornerRadius::same(4))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;

                // Header row of the element card: Drag Grip + Clean Badge Label + Clone / Remove
                ui.horizontal(|ui| {
                    let grip_resp = widgets::drag_handle(ui, loc.inspector_tip_drag_step);
                    if grip_resp.drag_started() {
                        drag_started_idx = Some(idx);
                    }

                    let (badge_text, badge_color) = element_badge(elem, t, loc);

                    // Clean badge label without selectable_label background highlight
                    let badge_resp = ui.add(
                        egui::Label::new(
                            RichText::new(badge_text)
                                .family(theme::semibold())
                                .size(10.5)
                                .color(badge_color)
                        ).sense(Sense::click())
                    ).on_hover_cursor(egui::CursorIcon::PointingHand);

                    if badge_resp.clicked() || grip_resp.clicked() {
                        *selected_idx = Some(idx);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::glyph_button(ui, Glyph::Close, loc.inspector_tip_remove_step).clicked() {
                            remove_idx = Some(idx);
                        }
                        if widgets::glyph_button(ui, Glyph::Clone, loc.inspector_tip_clone_step).clicked() {
                            clone_idx = Some(idx);
                        }
                    });
                });

                // Body rows of the element card, one panel per element kind (U02-011)
                let mut panel_ctx = ElementPanelCtx {
                    canvas_w,
                    canvas_h,
                    click_through,
                    active_color_target: &mut *active_color_target,
                    rules_list,
                    t,
                    loc,
                };
                render_element_panel(ui, idx, elem, &mut panel_ctx);
            });

        card_centers.push(card_resp.response.rect.center().y);
        ui.add_space(4.0);
    }

    // Determine target slot while dragging using fresh centers from this frame
    if let (Some(session), Some(pointer)) = (&mut dnd_session, pointer_pos) {
        if !card_centers.is_empty() {
            session.target_idx = dnd::slot_from_pointer(&card_centers, pointer.y);
        }
    }

    let active_dst = dnd_session.as_ref().map(|s| s.target_idx).or(active_dst);

    // Trailing drop slot after all elements
    if total_elems > 0 {
        render_elem_drop_slot(ui, total_elems, active_dst, active_src, total_elems, t);

        if widgets::full_width_action_button(ui, loc.overlay_studio_btn_add_element_large, "", t) {
            *open_element_catalog = true;
        }
        ui.add_space(4.0);
    }

    // Commit reorder on mouse release
    if pointer_released {
        if let Some(session) = dnd_session {
            let src = session.source_idx;
            let dst = session.target_idx;
            if src < overlay.elements.len() && dst <= overlay.elements.len() {
                if dst < src {
                    let snapshot = overlay.clone();
                    push_overlay_undo(undo_stack, redo_stack, &snapshot);
                    let item = overlay.elements.remove(src);
                    overlay.elements.insert(dst, item);
                    *selected_idx = Some(dst);
                } else if dst > src + 1 {
                    let snapshot = overlay.clone();
                    push_overlay_undo(undo_stack, redo_stack, &snapshot);
                    let item = overlay.elements.remove(src);
                    overlay.elements.insert(dst - 1, item);
                    *selected_idx = Some(dst - 1);
                }
            }
            dnd_session = None;
            ui.ctx().data_mut(|d| d.remove_temp::<ElementDndSession>(dnd_id));
        }
    }

    if let Some(src_idx) = drag_started_idx {
        dnd_session = Some(ElementDndSession {
            source_idx: src_idx,
            target_idx: src_idx,
        });
    }

    if let Some(session) = dnd_session {
        ui.ctx().data_mut(|d| d.insert_temp(dnd_id, session));
        ui.ctx().request_repaint();
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);

        // Floating drag preview badge following the pointer
        if let Some(pointer) = pointer_pos {
            if session.source_idx < overlay.elements.len() {
                let (badge_text, _) = element_badge(&overlay.elements[session.source_idx], t, loc);
                let float_rect = Rect::from_min_size(pointer + vec2(12.0, 10.0), vec2(110.0, 26.0));
                dnd::floating_preview(
                    ui,
                    egui::Id::unique("elem_drag_float"),
                    float_rect,
                    &ELEM_PREVIEW_STYLE,
                    t,
                    |p| {
                        p.text(
                            float_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            badge_text,
                            FontId::new(11.0, theme::semibold()),
                            t.ink,
                        );
                    },
                );
            }
        }
    }

    // Handle cloning
    if let Some(idx) = clone_idx {
        if idx < overlay.elements.len() {
            let snapshot = overlay.clone();
            push_overlay_undo(undo_stack, redo_stack, &snapshot);
            let mut cloned = overlay.elements[idx].clone();
            let (ew, eh) = get_element_size(&cloned);
            *cloned.id_mut() = format!("{}_copy", cloned.id());
            let cand_x = cloned.x() + 16.0;
            let cand_y = cloned.y() + 16.0;
            if cand_x + ew > overlay.rect.width || cand_y + eh > overlay.rect.height {
                *cloned.x_mut() = 0.0;
                *cloned.y_mut() = 0.0;
            } else {
                *cloned.x_mut() = cand_x;
                *cloned.y_mut() = cand_y;
            }
            overlay.elements.insert(idx + 1, cloned);
            clamp_elements_to_canvas(overlay);
            *selected_idx = Some(idx + 1);
        }
    }

    // Handle removal
    if let Some(r_idx) = remove_idx {
        if r_idx < overlay.elements.len() {
            let snapshot = overlay.clone();
            push_overlay_undo(undo_stack, redo_stack, &snapshot);
            overlay.elements.remove(r_idx);
            if *selected_idx == Some(r_idx) {
                *selected_idx = None;
            } else if let Some(sel) = *selected_idx {
                if sel > r_idx {
                    *selected_idx = Some(sel - 1);
                }
            }
        }
    }

    ui.add_space(4.0);
}

/// Dispatches one element card body to its per-kind panel.
fn render_element_panel(ui: &mut Ui, elem_idx: usize, elem: &mut OverlayElement, ctx: &mut ElementPanelCtx<'_>) {
    match elem {
        OverlayElement::Crosshair { .. } => render_crosshair_panel(ui, elem_idx, elem, ctx),
        OverlayElement::Text { .. } => render_text_panel(ui, elem_idx, elem, ctx),
        OverlayElement::Image { .. } => render_image_panel(ui, elem_idx, elem, ctx),
        OverlayElement::Shape { .. } => render_shape_panel(ui, elem_idx, elem, ctx),
        OverlayElement::Graph { .. } => render_graph_panel(ui, elem_idx, elem, ctx),
        OverlayElement::ClockTimer { .. } => render_clock_panel(ui, elem_idx, elem, ctx),
        OverlayElement::InteractiveControl { .. } => render_control_panel(ui, elem_idx, elem, ctx),
        OverlayElement::SystemTelemetry { .. } => render_telemetry_panel(ui, elem_idx, elem, ctx),
        OverlayElement::MediaWidget { .. } => render_media_panel(ui, elem_idx, elem, ctx),
        OverlayElement::KeyVisualizer { .. } => render_key_visualizer_panel(ui, elem_idx, elem, ctx),
    }
}

fn render_crosshair_panel(ui: &mut Ui, elem_idx: usize, elem: &mut OverlayElement, ctx: &mut ElementPanelCtx<'_>) {
    let OverlayElement::Crosshair { style, size, color, thickness, gap, center_dot, x, y, rotation, .. } = elem else {
        return;
    };
    let t = ctx.t;
    let loc = ctx.loc;
    let styles = [
        (CrosshairStyle::Cross, loc.overlay_studio_crosshair_cross),
        (CrosshairStyle::TShaped, loc.overlay_studio_crosshair_t),
        (CrosshairStyle::Circle, loc.overlay_studio_crosshair_circle),
        (CrosshairStyle::Dot, loc.overlay_studio_crosshair_dot),
    ];
    ui.horizontal(|ui| {
        for (s, lbl) in styles {
            let is_s = *style == s;
            let kind = if is_s { Kind::Primary } else { Kind::Secondary };
            if widgets::button(ui, lbl, kind, 0.0).clicked() {
                *style = s;
            }
        }
    });

    render_color_chip(ui, loc.overlay_studio_chip_color, color, element_color_target(elem_idx, ElementColorSlot::Primary), ctx.active_color_target, t);
    if active_element_slot(*ctx.active_color_target, elem_idx) == Some(ElementColorSlot::Primary) {
        render_inline_color_editor(ui, color, false, t, loc);
    }

    ui.columns(3, |cols| {
        cols[0].add(egui::DragValue::new(size).range(4.0..=128.0).prefix(loc.overlay_studio_field_size));
        cols[1].add(egui::DragValue::new(thickness).range(1.0..=8.0).prefix(loc.overlay_studio_field_thick));
        cols[2].add(egui::DragValue::new(gap).range(0.0..=32.0).prefix(loc.overlay_studio_field_gap));
    });
    ui.checkbox(center_dot, loc.overlay_studio_crosshair_center_dot);

    render_rotation_control(ui, rotation, t, loc);

    ui.columns(2, |cols| {
        cols[0].add(egui::DragValue::new(x).range(crosshair_coord_range(ctx.canvas_w)).prefix(loc.overlay_studio_field_x).suffix(loc.overlay_studio_unit_px));
        cols[1].add(egui::DragValue::new(y).range(crosshair_coord_range(ctx.canvas_h)).prefix(loc.overlay_studio_field_y).suffix(loc.overlay_studio_unit_px));
    });
}

fn render_text_panel(ui: &mut Ui, elem_idx: usize, elem: &mut OverlayElement, ctx: &mut ElementPanelCtx<'_>) {
    let OverlayElement::Text {
        text,
        font_size,
        color,
        x,
        y,
        bold,
        font_family,
        rotation,
        on_click,
        ..
    } = elem else {
        return;
    };
    let t = ctx.t;
    let loc = ctx.loc;
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(ui.available_width())
            .hint_text(loc.overlay_studio_hint_text)
    );

    // Font Family ComboBox
    let font_opts = [
        "Inter",
        "Archivo",
        "Geist Mono",
        "Segoe UI",
        "Consolas",
        "Arial",
        "Impact",
        "Tahoma",
        "Verdana",
        "Comic Sans MS",
        "Trebuchet MS",
        "Georgia",
        "Courier New",
    ];
    egui::ComboBox::from_id_salt(ui.scope_id().with(("font_combo", elem_idx)))
        .selected_text(font_family.as_str())
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for f in font_opts {
                ui.selectable_value(font_family, f.to_string(), f);
            }
        });

    render_color_chip(ui, loc.overlay_studio_chip_text_color, color, element_color_target(elem_idx, ElementColorSlot::Primary), ctx.active_color_target, t);
    if active_element_slot(*ctx.active_color_target, elem_idx) == Some(ElementColorSlot::Primary) {
        render_inline_color_editor(ui, color, true, t, loc);
    }

    ui.columns(2, |cols| {
        cols[0].add(egui::DragValue::new(font_size).range(8.0..=72.0).prefix(loc.overlay_studio_field_size).suffix(loc.overlay_studio_unit_pt));
        cols[1].checkbox(bold, loc.overlay_studio_text_bold);
    });

    render_rotation_control(ui, rotation, t, loc);

    let (nat_w, nat_h) = text_size_estimate(text, *font_size);
    render_position_rows(ui, x, y, nat_w, nat_h, ctx.canvas_w, ctx.canvas_h, loc);

    // On-Click Action
    if !ctx.click_through {
        render_element_click_action_ui(ui, on_click, ctx.rules_list, elem_idx, t, loc);
    }
}

fn render_image_panel(ui: &mut Ui, elem_idx: usize, elem: &mut OverlayElement, ctx: &mut ElementPanelCtx<'_>) {
    let OverlayElement::Image { path, x, y, width, height, opacity, lock_aspect, rotation, on_click, .. } = elem else {
        return;
    };
    let t = ctx.t;
    let loc = ctx.loc;
    ui.horizontal(|ui| {
        if widgets::button(ui, loc.overlay_studio_btn_browse, Kind::Secondary, 0.0).clicked() {
            if let Some(picked) = rfd::FileDialog::new()
                .add_filter(loc.overlay_studio_file_filter_image, &["png", "jpg", "jpeg", "bmp", "ico", "webp"])
                .pick_file()
            {
                *path = picked.to_string_lossy().to_string();
                if let Ok(img) = image::open(&picked) {
                    let (nw, nh) = (img.width() as f32, img.height() as f32);
                    if nw > 0.0 && nh > 0.0 {
                        let max_w = (ctx.canvas_w - 16.0).max(8.0);
                        let max_h = (ctx.canvas_h - 16.0).max(8.0);
                        if nw > max_w || nh > max_h {
                            let scale_factor = (max_w / nw).min(max_h / nh);
                            *width = (nw * scale_factor).round().max(8.0);
                            *height = (nh * scale_factor).round().max(8.0);
                        } else {
                            *width = nw;
                            *height = nh;
                        }
                    }
                }
            }
        }
        let img_path_resp = ui.add(
            egui::TextEdit::singleline(path)
                .desired_width(ui.available_width())
                .min_size(vec2(0.0, theme::CONTROL_H))
                .margin(vec2(8.0, 5.0))
                .hint_text(loc.overlay_studio_hint_img_path),
        );
        if img_path_resp.changed() || img_path_resp.lost_focus() {
            let clean = path.trim().trim_matches('"').trim_matches('\'').to_string();
            if clean != *path {
                *path = clean;
            }
        }
    });

    // Width & Height with Aspect Ratio Lock
    render_size_rows(
        ui,
        width,
        height,
        Some(lock_aspect),
        ctx.canvas_w,
        ctx.canvas_h,
        IMAGE_SIZE_LIMITS,
        (loc.overlay_studio_img_lock_aspect, loc.overlay_studio_img_unlock_aspect),
        loc,
        t,
    );

    ui.add(
        egui::Slider::new(opacity, 0.0..=1.0)
            .text(loc.overlay_studio_elem_img_opacity)
            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
    );

    render_rotation_control(ui, rotation, t, loc);

    render_position_rows(ui, x, y, *width, *height, ctx.canvas_w, ctx.canvas_h, loc);

    if !ctx.click_through {
        render_element_click_action_ui(ui, on_click, ctx.rules_list, elem_idx, t, loc);
    }
}

fn render_shape_panel(ui: &mut Ui, elem_idx: usize, elem: &mut OverlayElement, ctx: &mut ElementPanelCtx<'_>) {
    let OverlayElement::Shape {
        shape_type,
        x,
        y,
        width,
        height,
        thickness,
        color,
        filled,
        fill_color,
        corner_radius,
        lock_aspect,
        rotation,
        on_click,
        ..
    } = elem else {
        return;
    };
    let t = ctx.t;
    let loc = ctx.loc;
    let shapes = [
        (ShapeKind::Rectangle, loc.overlay_studio_shape_rect),
        (ShapeKind::Circle, loc.overlay_studio_shape_circle),
        (ShapeKind::Arrow, loc.overlay_studio_shape_arrow),
        (ShapeKind::Line, loc.overlay_studio_shape_line),
    ];
    ui.horizontal(|ui| {
        for (s, lbl) in shapes {
            let is_s = *shape_type == s;
            let kind = if is_s { Kind::Primary } else { Kind::Secondary };
            if widgets::button(ui, lbl, kind, 0.0).clicked() {
                *shape_type = s;
            }
        }
    });

    let is_fillable = *shape_type == ShapeKind::Rectangle || *shape_type == ShapeKind::Circle;
    if is_fillable {
        ui.columns(2, |cols| {
            render_color_chip(&mut cols[0], loc.overlay_studio_chip_color, color, element_color_target(elem_idx, ElementColorSlot::Primary), ctx.active_color_target, t);
            if *filled {
                render_color_chip(&mut cols[1], loc.overlay_studio_chip_fill, fill_color, element_color_target(elem_idx, ElementColorSlot::Secondary), ctx.active_color_target, t);
            } else {
                cols[1].checkbox(filled, loc.overlay_studio_shape_fill);
            }
        });
        if *filled {
            ui.checkbox(filled, loc.overlay_studio_shape_fill);
        }
    } else {
        render_color_chip(ui, loc.overlay_studio_chip_color, color, element_color_target(elem_idx, ElementColorSlot::Primary), ctx.active_color_target, t);
    }

    match active_element_slot(*ctx.active_color_target, elem_idx) {
        Some(ElementColorSlot::Primary) => render_inline_color_editor(ui, color, true, t, loc),
        Some(ElementColorSlot::Secondary) if is_fillable => render_inline_color_editor(ui, fill_color, true, t, loc),
        _ => {}
    }

    let can_round = *shape_type == ShapeKind::Rectangle || *shape_type == ShapeKind::Line;
    if can_round {
        ui.columns(2, |cols| {
            cols[0].add(egui::DragValue::new(thickness).range(1.0..=32.0).prefix(loc.overlay_studio_field_thick).suffix(loc.overlay_studio_unit_px));
            cols[1].add(egui::DragValue::new(corner_radius).range(0.0..=64.0).prefix(loc.overlay_studio_field_radius).suffix(loc.overlay_studio_unit_px));
        });
    } else {
        ui.add(egui::DragValue::new(thickness).range(1.0..=32.0).prefix(loc.overlay_studio_field_thick).suffix(loc.overlay_studio_unit_px));
    }

    render_size_rows(
        ui,
        width,
        height,
        Some(lock_aspect),
        ctx.canvas_w,
        ctx.canvas_h,
        SHAPE_SIZE_LIMITS,
        (loc.overlay_studio_img_lock_aspect, loc.overlay_studio_img_unlock_aspect),
        loc,
        t,
    );

    render_rotation_control(ui, rotation, t, loc);

    render_position_rows(ui, x, y, *width, *height, ctx.canvas_w, ctx.canvas_h, loc);

    if !ctx.click_through {
        render_element_click_action_ui(ui, on_click, ctx.rules_list, elem_idx, t, loc);
    }
}

fn render_graph_panel(ui: &mut Ui, elem_idx: usize, elem: &mut OverlayElement, ctx: &mut ElementPanelCtx<'_>) {
    let OverlayElement::Graph {
        metric,
        style,
        sample_count,
        color,
        fill_color,
        bg_color,
        show_current_val,
        x,
        y,
        width,
        height,
        rotation,
        ..
    } = elem else {
        return;
    };
    let t = ctx.t;
    let loc = ctx.loc;
    ui.horizontal(|ui| {
        ui.label(RichText::new(loc.overlay_studio_graph_metric).size(10.0).family(theme::semibold()).color(t.faint));
        let metric_lbl = match metric {
            crate::models::GraphMetric::SystemCpu => loc.overlay_studio_graph_metric_cpu,
            crate::models::GraphMetric::ProcessCpu => loc.overlay_studio_graph_metric_proc_cpu,
            crate::models::GraphMetric::SystemRam => loc.overlay_studio_graph_metric_ram,
            crate::models::GraphMetric::ProcessRam => loc.overlay_studio_graph_metric_proc_ram,
            crate::models::GraphMetric::Fps => loc.overlay_studio_graph_metric_fps,
            crate::models::GraphMetric::Battery => loc.overlay_studio_graph_metric_battery,
            crate::models::GraphMetric::Network => loc.overlay_studio_graph_metric_network,
        };
        egui::ComboBox::from_id_salt(ui.scope_id().with(("graph_metric", elem_idx)))
            .selected_text(metric_lbl)
            .width(ui.available_width() - 8.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(metric, crate::models::GraphMetric::SystemCpu, loc.overlay_studio_graph_metric_cpu);
                ui.selectable_value(metric, crate::models::GraphMetric::ProcessCpu, loc.overlay_studio_graph_metric_proc_cpu);
                ui.selectable_value(metric, crate::models::GraphMetric::SystemRam, loc.overlay_studio_graph_metric_ram);
                ui.selectable_value(metric, crate::models::GraphMetric::ProcessRam, loc.overlay_studio_graph_metric_proc_ram);
                ui.selectable_value(metric, crate::models::GraphMetric::Fps, loc.overlay_studio_graph_metric_fps);
                ui.selectable_value(metric, crate::models::GraphMetric::Battery, loc.overlay_studio_graph_metric_battery);
                ui.selectable_value(metric, crate::models::GraphMetric::Network, loc.overlay_studio_graph_metric_network);
            });
    });

    ui.horizontal(|ui| {
        let styles = [
            (crate::models::GraphStyle::AreaFilled, loc.overlay_studio_graph_style_area),
            (crate::models::GraphStyle::Line, loc.overlay_studio_graph_style_line),
            (crate::models::GraphStyle::Histogram, loc.overlay_studio_graph_style_histogram),
        ];
        for (s, lbl) in styles {
            let is_s = *style == s;
            let kind = if is_s { Kind::Primary } else { Kind::Secondary };
            if widgets::button(ui, lbl, kind, 0.0).clicked() {
                *style = s;
            }
        }
    });

    ui.add(egui::Slider::new(sample_count, 10..=120).text(loc.overlay_studio_graph_samples));
    ui.checkbox(show_current_val, loc.overlay_studio_graph_show_val);

    ui.columns(3, |cols| {
        render_color_chip(&mut cols[0], loc.overlay_studio_chip_line, color, element_color_target(elem_idx, ElementColorSlot::Primary), ctx.active_color_target, t);
        render_color_chip(&mut cols[1], loc.overlay_studio_chip_fill, fill_color, element_color_target(elem_idx, ElementColorSlot::Secondary), ctx.active_color_target, t);
        render_color_chip(&mut cols[2], loc.overlay_studio_chip_bg, bg_color, element_color_target(elem_idx, ElementColorSlot::Tertiary), ctx.active_color_target, t);
    });
    match active_element_slot(*ctx.active_color_target, elem_idx) {
        Some(ElementColorSlot::Primary) => render_inline_color_editor(ui, color, true, t, loc),
        Some(ElementColorSlot::Secondary) => render_inline_color_editor(ui, fill_color, true, t, loc),
        Some(ElementColorSlot::Tertiary) => render_inline_color_editor(ui, bg_color, true, t, loc),
        None => {}
    }

    render_size_rows(ui, width, height, None, ctx.canvas_w, ctx.canvas_h, PANEL_SIZE_LIMITS, ("", ""), loc, t);

    render_rotation_control(ui, rotation, t, loc);

    render_position_rows(ui, x, y, *width, *height, ctx.canvas_w, ctx.canvas_h, loc);
}

fn render_clock_panel(ui: &mut Ui, elem_idx: usize, elem: &mut OverlayElement, ctx: &mut ElementPanelCtx<'_>) {
    let OverlayElement::ClockTimer {
        mode,
        format_str,
        font_size,
        color,
        bg_color,
        countdown_total_ms,
        bold,
        x,
        y,
        width,
        height,
        rotation,
        on_finish,
        ..
    } = elem else {
        return;
    };
    let t = ctx.t;
    let loc = ctx.loc;
    ui.horizontal(|ui| {
        ui.label(RichText::new(loc.overlay_studio_timer_mode).size(10.0).family(theme::semibold()).color(t.faint));
        let mode_lbl = match mode {
            crate::models::TimerMode::Clock => loc.overlay_studio_timer_mode_clock,
            crate::models::TimerMode::Stopwatch => loc.overlay_studio_timer_mode_stopwatch,
            crate::models::TimerMode::Countdown => loc.overlay_studio_timer_mode_countdown,
            crate::models::TimerMode::Uptime => loc.overlay_studio_timer_mode_uptime,
        };
        egui::ComboBox::from_id_salt(ui.scope_id().with(("clock_mode", elem_idx)))
            .selected_text(mode_lbl)
            .width(ui.available_width() - 8.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(mode, crate::models::TimerMode::Clock, loc.overlay_studio_timer_mode_clock);
                ui.selectable_value(mode, crate::models::TimerMode::Stopwatch, loc.overlay_studio_timer_mode_stopwatch);
                ui.selectable_value(mode, crate::models::TimerMode::Countdown, loc.overlay_studio_timer_mode_countdown);
                ui.selectable_value(mode, crate::models::TimerMode::Uptime, loc.overlay_studio_timer_mode_uptime);
            });
    });

    match mode {
        crate::models::TimerMode::Clock => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(loc.overlay_studio_timer_format).size(10.0).family(theme::semibold()).color(t.faint));
                ui.text_edit_singleline(format_str);
            });
        }
        crate::models::TimerMode::Countdown => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(loc.overlay_studio_timer_countdown).size(10.0).family(theme::semibold()).color(t.faint));
                let mut total_ms = *countdown_total_ms;
                if widgets::duration_millis_u64(ui, loc, ("timer_countdown", elem_idx), &mut total_ms, 1000.0..=86_400_000.0).changed() {
                    *countdown_total_ms = total_ms;
                }
            });
        }
        _ => {}
    }

    ui.horizontal(|ui| {
        ui.add(egui::Slider::new(font_size, 8.0..=48.0).text(loc.overlay_studio_font_size));
        ui.checkbox(bold, loc.overlay_studio_text_bold);
    });

    ui.columns(2, |cols| {
        render_color_chip(&mut cols[0], loc.overlay_studio_chip_text, color, element_color_target(elem_idx, ElementColorSlot::Primary), ctx.active_color_target, t);
        render_color_chip(&mut cols[1], loc.overlay_studio_chip_bg, bg_color, element_color_target(elem_idx, ElementColorSlot::Secondary), ctx.active_color_target, t);
    });
    match active_element_slot(*ctx.active_color_target, elem_idx) {
        Some(ElementColorSlot::Primary) => render_inline_color_editor(ui, color, true, t, loc),
        Some(ElementColorSlot::Secondary) => render_inline_color_editor(ui, bg_color, true, t, loc),
        _ => {}
    }

    render_size_rows(ui, width, height, None, ctx.canvas_w, ctx.canvas_h, PANEL_SIZE_LIMITS, ("", ""), loc, t);

    render_rotation_control(ui, rotation, t, loc);

    render_position_rows(ui, x, y, *width, *height, ctx.canvas_w, ctx.canvas_h, loc);

    if !ctx.click_through && matches!(mode, crate::models::TimerMode::Countdown) {
        render_element_click_action_ui(ui, on_finish, ctx.rules_list, elem_idx, t, loc);
    }
}

/// Interactive-control kinds offered by the inspector's kind buttons. Kept separate from the
/// user-visible labels so switching language cannot change which kind a button selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlKindChoice {
    Button,
    Toggle,
    Slider,
}

fn render_control_panel(ui: &mut Ui, elem_idx: usize, elem: &mut OverlayElement, ctx: &mut ElementPanelCtx<'_>) {
    let OverlayElement::InteractiveControl { control, color, bg_color, active_color, x, y, width, height, rotation, .. } = elem else {
        return;
    };
    let t = ctx.t;
    let loc = ctx.loc;
    let cur_kind = match control {
        crate::models::InteractiveControlKind::Button { .. } => ControlKindChoice::Button,
        crate::models::InteractiveControlKind::ToggleSwitch { .. } => ControlKindChoice::Toggle,
        crate::models::InteractiveControlKind::Slider { .. } => ControlKindChoice::Slider,
    };

    ui.horizontal(|ui| {
        ui.label(RichText::new(loc.overlay_studio_control_kind).size(10.0).family(theme::semibold()).color(t.faint));
        let kinds = [
            (loc.overlay_studio_control_btn, ControlKindChoice::Button),
            (loc.overlay_studio_control_toggle, ControlKindChoice::Toggle),
            (loc.overlay_studio_control_slider, ControlKindChoice::Slider),
        ];
        for (lbl, kind_choice) in kinds {
            let is_sel = cur_kind == kind_choice;
            let kind = if is_sel { Kind::Primary } else { Kind::Secondary };
            if widgets::button(ui, lbl, kind, 0.0).clicked() && !is_sel {
                *control = match kind_choice {
                    ControlKindChoice::Button => crate::models::InteractiveControlKind::Button {
                        label: loc.overlay_studio_control_default_action.to_string(),
                        icon: None,
                        action: None,
                    },
                    ControlKindChoice::Toggle => crate::models::InteractiveControlKind::ToggleSwitch {
                        label: loc.overlay_studio_control_toggle.to_string(),
                        is_active: false,
                        bind_rule_id: None,
                        on_enable: None,
                        on_disable: None,
                    },
                    ControlKindChoice::Slider => crate::models::InteractiveControlKind::Slider {
                        label: loc.overlay_studio_control_default_volume.to_string(),
                        value: 50.0,
                        min: 0.0,
                        max: 100.0,
                        bind_target: crate::models::SliderBindTarget::MasterVolume,
                    },
                };
            }
        }
    });

    match control {
        crate::models::InteractiveControlKind::Button { label, action, .. } => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(loc.overlay_studio_control_label).size(10.0).family(theme::semibold()).color(t.faint));
                ui.text_edit_singleline(label);
            });
            render_element_click_action_ui(ui, action, ctx.rules_list, elem_idx, t, loc);
        }
        crate::models::InteractiveControlKind::ToggleSwitch { label, is_active, on_enable, on_disable, .. } => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(loc.overlay_studio_control_label).size(10.0).family(theme::semibold()).color(t.faint));
                ui.text_edit_singleline(label);
            });
            ui.checkbox(is_active, loc.overlay_studio_control_initial_state);
            render_element_click_action_ui(ui, on_enable, ctx.rules_list, elem_idx + 100, t, loc);
            render_element_click_action_ui(ui, on_disable, ctx.rules_list, elem_idx + 200, t, loc);
        }
        crate::models::InteractiveControlKind::Slider { label, value, min, max, bind_target } => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(loc.overlay_studio_control_label).size(10.0).family(theme::semibold()).color(t.faint));
                ui.text_edit_singleline(label);
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(loc.overlay_studio_control_slider_target).size(10.0).family(theme::semibold()).color(t.faint));
                let target_lbl = match bind_target {
                    crate::models::SliderBindTarget::MasterVolume => loc.overlay_studio_slider_master_volume,
                    crate::models::SliderBindTarget::CustomVariable => loc.overlay_studio_slider_custom_variable,
                    crate::models::SliderBindTarget::OverlayOpacity => loc.overlay_studio_slider_overlay_opacity,
                    crate::models::SliderBindTarget::ProcessVolume => loc.overlay_studio_slider_process_volume,
                };
                egui::ComboBox::from_id_salt(ui.scope_id().with(("slider_target", elem_idx)))
                    .selected_text(target_lbl)
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(bind_target, crate::models::SliderBindTarget::MasterVolume, loc.overlay_studio_slider_master_volume);
                        ui.selectable_value(bind_target, crate::models::SliderBindTarget::CustomVariable, loc.overlay_studio_slider_custom_variable);
                        ui.selectable_value(bind_target, crate::models::SliderBindTarget::OverlayOpacity, loc.overlay_studio_slider_overlay_opacity);
                        ui.selectable_value(bind_target, crate::models::SliderBindTarget::ProcessVolume, loc.overlay_studio_slider_process_volume);
                    });
            });
            ui.columns(2, |cols| {
                cols[0].add(egui::DragValue::new(min).prefix(loc.overlay_studio_field_min));
                cols[1].add(egui::DragValue::new(max).prefix(loc.overlay_studio_field_max));
            });
            ui.add(egui::Slider::new(value, *min..=*max).text(loc.overlay_studio_control_label));
        }
    }

    ui.columns(3, |cols| {
        render_color_chip(&mut cols[0], loc.overlay_studio_chip_text, color, element_color_target(elem_idx, ElementColorSlot::Primary), ctx.active_color_target, t);
        render_color_chip(&mut cols[1], loc.overlay_studio_chip_bg, bg_color, element_color_target(elem_idx, ElementColorSlot::Secondary), ctx.active_color_target, t);
        render_color_chip(&mut cols[2], loc.overlay_studio_chip_active, active_color, element_color_target(elem_idx, ElementColorSlot::Tertiary), ctx.active_color_target, t);
    });
    match active_element_slot(*ctx.active_color_target, elem_idx) {
        Some(ElementColorSlot::Primary) => render_inline_color_editor(ui, color, true, t, loc),
        Some(ElementColorSlot::Secondary) => render_inline_color_editor(ui, bg_color, true, t, loc),
        Some(ElementColorSlot::Tertiary) => render_inline_color_editor(ui, active_color, true, t, loc),
        None => {}
    }

    render_size_rows(ui, width, height, None, ctx.canvas_w, ctx.canvas_h, PANEL_SIZE_LIMITS, ("", ""), loc, t);

    render_rotation_control(ui, rotation, t, loc);

    render_position_rows(ui, x, y, *width, *height, ctx.canvas_w, ctx.canvas_h, loc);
}

fn render_telemetry_panel(ui: &mut Ui, elem_idx: usize, elem: &mut OverlayElement, ctx: &mut ElementPanelCtx<'_>) {
    let OverlayElement::SystemTelemetry {
        kind,
        style,
        color,
        bg_color,
        x,
        y,
        width,
        height,
        rotation,
        on_click,
        ..
    } = elem else {
        return;
    };
    let t = ctx.t;
    let loc = ctx.loc;
    ui.horizontal(|ui| {
        ui.label(RichText::new(loc.overlay_studio_telemetry_kind).size(10.0).family(theme::semibold()).color(t.faint));
        let kind_lbl = match kind {
            crate::models::TelemetryKind::Battery => loc.overlay_studio_telemetry_kind_battery,
            crate::models::TelemetryKind::AudioMicStatus => loc.overlay_studio_telemetry_kind_audio,
            crate::models::TelemetryKind::EngineStatus => loc.overlay_studio_telemetry_kind_engine,
        };
        egui::ComboBox::from_id_salt(ui.scope_id().with(("telem_kind", elem_idx)))
            .selected_text(kind_lbl)
            .width(ui.available_width() - 8.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(kind, crate::models::TelemetryKind::Battery, loc.overlay_studio_telemetry_kind_battery);
                ui.selectable_value(kind, crate::models::TelemetryKind::AudioMicStatus, loc.overlay_studio_telemetry_kind_audio);
                ui.selectable_value(kind, crate::models::TelemetryKind::EngineStatus, loc.overlay_studio_telemetry_kind_engine);
            });
    });

    ui.horizontal(|ui| {
        ui.label(RichText::new(loc.overlay_studio_telemetry_style).size(10.0).family(theme::semibold()).color(t.faint));
        let style_lbl = match style {
            crate::models::TelemetryStyle::Badge => loc.overlay_studio_telemetry_style_badge,
            crate::models::TelemetryStyle::Compact => loc.overlay_studio_telemetry_style_compact,
            crate::models::TelemetryStyle::Detailed => loc.overlay_studio_telemetry_style_detailed,
        };
        egui::ComboBox::from_id_salt(ui.scope_id().with(("telem_style", elem_idx)))
            .selected_text(style_lbl)
            .width(ui.available_width() - 8.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(style, crate::models::TelemetryStyle::Badge, loc.overlay_studio_telemetry_style_badge);
                ui.selectable_value(style, crate::models::TelemetryStyle::Compact, loc.overlay_studio_telemetry_style_compact);
                ui.selectable_value(style, crate::models::TelemetryStyle::Detailed, loc.overlay_studio_telemetry_style_detailed);
            });
    });

    ui.columns(2, |cols| {
        render_color_chip(&mut cols[0], loc.overlay_studio_chip_color, color, element_color_target(elem_idx, ElementColorSlot::Primary), ctx.active_color_target, t);
        render_color_chip(&mut cols[1], loc.overlay_studio_chip_bg, bg_color, element_color_target(elem_idx, ElementColorSlot::Secondary), ctx.active_color_target, t);
    });
    match active_element_slot(*ctx.active_color_target, elem_idx) {
        Some(ElementColorSlot::Primary) => render_inline_color_editor(ui, color, true, t, loc),
        Some(ElementColorSlot::Secondary) => render_inline_color_editor(ui, bg_color, true, t, loc),
        _ => {}
    }

    render_size_rows(ui, width, height, None, ctx.canvas_w, ctx.canvas_h, PANEL_SIZE_LIMITS, ("", ""), loc, t);

    render_rotation_control(ui, rotation, t, loc);

    render_position_rows(ui, x, y, *width, *height, ctx.canvas_w, ctx.canvas_h, loc);

    if !ctx.click_through {
        render_element_click_action_ui(ui, on_click, ctx.rules_list, elem_idx, t, loc);
    }
}

fn render_media_panel(ui: &mut Ui, elem_idx: usize, elem: &mut OverlayElement, ctx: &mut ElementPanelCtx<'_>) {
    let OverlayElement::MediaWidget { show_controls, compact, color, bg_color, x, y, width, height, rotation, .. } = elem else {
        return;
    };
    let t = ctx.t;
    let loc = ctx.loc;
    ui.horizontal(|ui| {
        ui.checkbox(show_controls, loc.overlay_studio_media_controls);
        ui.checkbox(compact, loc.overlay_studio_compact_mode);
    });

    ui.columns(2, |cols| {
        render_color_chip(&mut cols[0], loc.overlay_studio_chip_text, color, element_color_target(elem_idx, ElementColorSlot::Primary), ctx.active_color_target, t);
        render_color_chip(&mut cols[1], loc.overlay_studio_chip_bg, bg_color, element_color_target(elem_idx, ElementColorSlot::Secondary), ctx.active_color_target, t);
    });
    match active_element_slot(*ctx.active_color_target, elem_idx) {
        Some(ElementColorSlot::Primary) => render_inline_color_editor(ui, color, true, t, loc),
        Some(ElementColorSlot::Secondary) => render_inline_color_editor(ui, bg_color, true, t, loc),
        _ => {}
    }

    render_size_rows(ui, width, height, None, ctx.canvas_w, ctx.canvas_h, MEDIA_SIZE_LIMITS, ("", ""), loc, t);

    render_rotation_control(ui, rotation, t, loc);

    render_position_rows(ui, x, y, *width, *height, ctx.canvas_w, ctx.canvas_h, loc);
}

fn render_key_visualizer_panel(ui: &mut Ui, elem_idx: usize, elem: &mut OverlayElement, ctx: &mut ElementPanelCtx<'_>) {
    let OverlayElement::KeyVisualizer {
        layout,
        max_keys,
        decay_seconds,
        show_modifiers_only,
        color,
        bg_color,
        active_color,
        x,
        y,
        width,
        height,
        rotation,
        ..
    } = elem else {
        return;
    };
    let t = ctx.t;
    let loc = ctx.loc;
    let mut changed_layout = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(loc.overlay_studio_keyvis_layout).size(10.0).family(theme::semibold()).color(t.faint));
        let selected_text = match layout {
            crate::models::KeyVisualizerLayout::WasdMouse => loc.overlay_studio_keyvis_wasd_mouse,
            crate::models::KeyVisualizerLayout::WasdOnly => loc.overlay_studio_keyvis_wasd_only,
            crate::models::KeyVisualizerLayout::ActionRpg => loc.overlay_studio_keyvis_action_rpg,
            crate::models::KeyVisualizerLayout::MouseOnly => loc.overlay_studio_keyvis_mouse_only,
            crate::models::KeyVisualizerLayout::TextStream => loc.overlay_studio_keyvis_text_stream,
        };
        egui::ComboBox::from_id_salt(format!("kv_layout_{elem_idx}"))
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                if ui.selectable_value(layout, crate::models::KeyVisualizerLayout::WasdMouse, loc.overlay_studio_keyvis_wasd_mouse).clicked() {
                    changed_layout = true;
                }
                if ui.selectable_value(layout, crate::models::KeyVisualizerLayout::WasdOnly, loc.overlay_studio_keyvis_wasd_only).clicked() {
                    changed_layout = true;
                }
                if ui.selectable_value(layout, crate::models::KeyVisualizerLayout::ActionRpg, loc.overlay_studio_keyvis_action_rpg).clicked() {
                    changed_layout = true;
                }
                if ui.selectable_value(layout, crate::models::KeyVisualizerLayout::MouseOnly, loc.overlay_studio_keyvis_mouse_only).clicked() {
                    changed_layout = true;
                }
                if ui.selectable_value(layout, crate::models::KeyVisualizerLayout::TextStream, loc.overlay_studio_keyvis_text_stream).clicked() {
                    changed_layout = true;
                }
            });
    });
    if changed_layout {
        match layout {
            crate::models::KeyVisualizerLayout::WasdMouse => { *width = 170.0; *height = 105.0; }
            crate::models::KeyVisualizerLayout::WasdOnly => { *width = 120.0; *height = 100.0; }
            crate::models::KeyVisualizerLayout::ActionRpg => { *width = 160.0; *height = 110.0; }
            crate::models::KeyVisualizerLayout::MouseOnly => { *width = 90.0; *height = 70.0; }
            crate::models::KeyVisualizerLayout::TextStream => { *width = 160.0; *height = 32.0; }
        }
    }

    if *layout == crate::models::KeyVisualizerLayout::TextStream {
        ui.horizontal(|ui| {
            ui.label(RichText::new(loc.overlay_studio_key_max).size(10.0).family(theme::semibold()).color(t.faint));
            ui.add(egui::DragValue::new(max_keys).range(1..=8));
            ui.label(RichText::new(loc.overlay_studio_key_decay).size(10.0).family(theme::semibold()).color(t.faint));
            widgets::duration_secs_f32(ui, loc, ("key_decay", elem_idx), decay_seconds, 0.2..=5.0);
        });
        ui.checkbox(show_modifiers_only, loc.overlay_studio_key_modifiers_only);
    }

    if *layout == crate::models::KeyVisualizerLayout::TextStream {
        ui.columns(2, |cols| {
            render_color_chip(&mut cols[0], loc.overlay_studio_chip_text, color, element_color_target(elem_idx, ElementColorSlot::Primary), ctx.active_color_target, t);
            render_color_chip(&mut cols[1], loc.overlay_studio_chip_bg, bg_color, element_color_target(elem_idx, ElementColorSlot::Secondary), ctx.active_color_target, t);
        });
    } else {
        ui.columns(3, |cols| {
            render_color_chip(&mut cols[0], loc.overlay_studio_chip_text, color, element_color_target(elem_idx, ElementColorSlot::Primary), ctx.active_color_target, t);
            render_color_chip(&mut cols[1], loc.overlay_studio_chip_bg, bg_color, element_color_target(elem_idx, ElementColorSlot::Secondary), ctx.active_color_target, t);
            render_color_chip(&mut cols[2], loc.overlay_studio_chip_glow, active_color, element_color_target(elem_idx, ElementColorSlot::Tertiary), ctx.active_color_target, t);
        });
    }

    match active_element_slot(*ctx.active_color_target, elem_idx) {
        Some(ElementColorSlot::Primary) => render_inline_color_editor(ui, color, true, t, loc),
        Some(ElementColorSlot::Secondary) => render_inline_color_editor(ui, bg_color, true, t, loc),
        Some(ElementColorSlot::Tertiary) => render_inline_color_editor(ui, active_color, true, t, loc),
        None => {}
    }

    render_size_rows(ui, width, height, None, ctx.canvas_w, ctx.canvas_h, KEYVISUALIZER_SIZE_LIMITS, ("", ""), loc, t);

    render_rotation_control(ui, rotation, t, loc);

    render_position_rows(ui, x, y, *width, *height, ctx.canvas_w, ctx.canvas_h, loc);
}

// ----------------- ELEMENT ON-CLICK ACTION UI (POINT 7) -----------------
/// Entry written when a rule is picked from the on-click list: the "Trigger" row starts the
/// rule, the "Toggle" row switches it. Both variants are executed by the renderer, so both must
/// stay authorable (U02-007). The toggle label comes from the catalog (`overlay_studio_on_click_toggle`).
fn rule_click_action(rule_id: &str, toggle: bool) -> OverlayClickAction {
    if toggle {
        OverlayClickAction::ToggleRule { rule_id: rule_id.to_string() }
    } else {
        OverlayClickAction::TriggerRule { rule_id: rule_id.to_string() }
    }
}

fn render_element_click_action_ui(
    ui: &mut Ui,
    on_click: &mut Option<OverlayClickAction>,
    rules_list: &[(String, String)],
    idx: usize,
    t: &Tokens,
    loc: &crate::i18n::Strings,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(loc.overlay_studio_on_click).size(10.0).family(theme::semibold()).color(t.faint));

        let current_lbl: String = match on_click {
            None => loc.overlay_studio_on_click_none.to_string(),
            Some(OverlayClickAction::CloseOverlay) => loc.overlay_studio_on_click_close.to_string(),
            Some(OverlayClickAction::TriggerRule { rule_id }) => {
                let name = rules_list.iter().find(|(id, _)| id == rule_id).map(|(_, n)| n.as_str()).unwrap_or(rule_id);
                format!("{}: {name}", loc.overlay_studio_on_click_trigger)
            }
            Some(OverlayClickAction::ToggleRule { rule_id }) => {
                let name = rules_list.iter().find(|(id, _)| id == rule_id).map(|(_, n)| n.as_str()).unwrap_or(rule_id);
                format!("{}: {name}", loc.overlay_studio_on_click_toggle)
            }
            Some(OverlayClickAction::KillProcess { process_name }) => {
                format!("{}: {process_name}", loc.overlay_studio_on_click_kill_prefix)
            }
            Some(OverlayClickAction::TrimMemory { process_name }) => {
                format!("{}: {process_name}", loc.overlay_studio_on_click_trim_prefix)
            }
            Some(OverlayClickAction::ToggleMute) => loc.overlay_studio_on_click_mute.to_string(),
            Some(OverlayClickAction::MediaPlayPause) => loc.overlay_studio_on_click_play_pause.to_string(),
            Some(OverlayClickAction::MediaNext) => loc.overlay_studio_on_click_next_track.to_string(),
            Some(OverlayClickAction::MediaPrev) => loc.overlay_studio_on_click_prev_track.to_string(),
            Some(OverlayClickAction::ResetTimer) => loc.overlay_studio_on_click_timer_reset.to_string(),
            Some(OverlayClickAction::ToggleTimer) => loc.overlay_studio_on_click_timer_toggle.to_string(),
        };

        egui::ComboBox::from_id_salt(ui.scope_id().with(("elem_onclick", idx)))
            .selected_text(current_lbl)
            .width(ui.available_width() - 8.0)
            .show_ui(ui, |ui| {
                if ui.selectable_label(on_click.is_none(), loc.overlay_studio_on_click_none).clicked() {
                    *on_click = None;
                }
                if ui.selectable_label(matches!(on_click, Some(OverlayClickAction::CloseOverlay)), loc.overlay_studio_on_click_close).clicked() {
                    *on_click = Some(OverlayClickAction::CloseOverlay);
                }
                if ui.selectable_label(matches!(on_click, Some(OverlayClickAction::ToggleMute)), loc.overlay_studio_on_click_mute).clicked() {
                    *on_click = Some(OverlayClickAction::ToggleMute);
                }
                if ui.selectable_label(matches!(on_click, Some(OverlayClickAction::MediaPlayPause)), loc.overlay_studio_on_click_play_pause).clicked() {
                    *on_click = Some(OverlayClickAction::MediaPlayPause);
                }
                if ui.selectable_label(matches!(on_click, Some(OverlayClickAction::MediaNext)), loc.overlay_studio_on_click_next_track).clicked() {
                    *on_click = Some(OverlayClickAction::MediaNext);
                }
                if ui.selectable_label(matches!(on_click, Some(OverlayClickAction::MediaPrev)), loc.overlay_studio_on_click_prev_track).clicked() {
                    *on_click = Some(OverlayClickAction::MediaPrev);
                }
                if ui.selectable_label(matches!(on_click, Some(OverlayClickAction::ToggleTimer)), loc.overlay_studio_on_click_timer_toggle).clicked() {
                    *on_click = Some(OverlayClickAction::ToggleTimer);
                }
                if ui.selectable_label(matches!(on_click, Some(OverlayClickAction::ResetTimer)), loc.overlay_studio_on_click_timer_reset).clicked() {
                    *on_click = Some(OverlayClickAction::ResetTimer);
                }
                if !rules_list.is_empty() {
                    ui.separator();
                    for (r_id, r_name) in rules_list {
                        let is_trigger = matches!(on_click, Some(OverlayClickAction::TriggerRule { rule_id }) if rule_id == r_id);
                        if ui.selectable_label(is_trigger, format!("{}: {r_name}", loc.overlay_studio_on_click_trigger)).clicked() {
                            *on_click = Some(rule_click_action(r_id, false));
                        }
                        // ToggleRule is executed by the renderer but used to be unauthorable
                        // here: both variants shared one entry that always wrote TriggerRule, so
                        // a loaded ToggleRule was silently downgraded on reselect (U02-007).
                        let is_toggle = matches!(on_click, Some(OverlayClickAction::ToggleRule { rule_id }) if rule_id == r_id);
                        if ui.selectable_label(is_toggle, format!("{}: {r_name}", loc.overlay_studio_on_click_toggle)).clicked() {
                            *on_click = Some(rule_click_action(r_id, true));
                        }
                    }
                }
            });
    });
}

// ----------------- INTERACTIVE VISUAL PREVIEW CANVAS (POINT 6) -----------------
#[derive(Debug, Clone, Copy, PartialEq)]
enum CanvasDragMode {
    None,
    Pan {
        start_pointer: egui::Pos2,
        start_pan: egui::Vec2,
    },
    MoveCard {
        start_pointer: egui::Pos2,
        start_card_sx: f32,
        start_card_sy: f32,
    },
    ResizeCard {
        start_pointer: egui::Pos2,
        start_w: f32,
        start_h: f32,
    },
    Move {
        idx: usize,
        start_x: f32,
        start_y: f32,
        start_pointer: egui::Pos2,
    },
    Resize {
        idx: usize,
        start_w: f32,
        start_h: f32,
        start_pointer: egui::Pos2,
    },
    Rotate {
        idx: usize,
        center: egui::Pos2,
        base_angle_rad: f32,
    },
}

pub fn resolve_overlay_monitor(
    overlay: &OverlayDefinition,
    monitors: &[crate::models::MonitorInfo],
) -> crate::models::MonitorInfo {
    overlay
        .monitor_index
        .and_then(|idx| monitors.get(idx))
        .or_else(|| monitors.iter().find(|m| m.is_primary))
        .or_else(|| monitors.first())
        .cloned()
        .unwrap_or_else(|| crate::models::MonitorInfo {
            index: 0,
            name: "Primary Display".to_string(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            is_primary: true,
        })
}

pub fn max_card_dimensions(overlay: &OverlayDefinition, screen_w: f32, screen_h: f32) -> (f32, f32) {
    let rx = overlay.rect.x.max(0.0);
    let ry = overlay.rect.y.max(0.0);
    match overlay.anchor {
        OverlayAnchor::ScreenCenter => {
            let mw = (screen_w - rx * 2.0).clamp(20.0, screen_w);
            let mh = (screen_h - ry * 2.0).clamp(20.0, screen_h);
            (mw, mh)
        }
        OverlayAnchor::TopLeft | OverlayAnchor::TopRight | OverlayAnchor::BottomLeft | OverlayAnchor::BottomRight => {
            ((screen_w - rx).clamp(20.0, screen_w), (screen_h - ry).clamp(20.0, screen_h))
        }
        OverlayAnchor::Custom => {
            ((screen_w - overlay.rect.x).clamp(20.0, screen_w), (screen_h - overlay.rect.y).clamp(20.0, screen_h))
        }
        OverlayAnchor::WindowRelative => (screen_w, screen_h),
    }
}

pub fn clamp_card_to_monitor(overlay: &mut OverlayDefinition, screen_w: f32, screen_h: f32) {
    let (max_w, max_h) = max_card_dimensions(overlay, screen_w, screen_h);
    overlay.rect.width = overlay.rect.width.clamp(20.0, max_w);
    overlay.rect.height = overlay.rect.height.clamp(20.0, max_h);
    if overlay.anchor == OverlayAnchor::Custom {
        overlay.rect.x = overlay.rect.x.clamp(0.0, (screen_w - overlay.rect.width).max(0.0));
        overlay.rect.y = overlay.rect.y.clamp(0.0, (screen_h - overlay.rect.height).max(0.0));
    }
}

/// Badge label and colour for one element kind, shared by the inspector card header and the
/// floating drag preview (U02-014).
fn element_badge(elem: &OverlayElement, t: &Tokens, loc: &crate::i18n::Strings) -> (&'static str, Color32) {
    match elem {
        OverlayElement::Crosshair { .. } => (loc.overlay_studio_badge_crosshair, t.accent_ink),
        OverlayElement::Text { .. } => (loc.overlay_studio_badge_text, t.ink),
        OverlayElement::Image { .. } => (loc.overlay_studio_badge_image, t.faint),
        OverlayElement::Shape { shape_type, .. } => (
            match shape_type {
                ShapeKind::Rectangle => loc.overlay_studio_badge_square,
                ShapeKind::Circle => loc.overlay_studio_badge_circle,
                ShapeKind::Arrow => loc.overlay_studio_badge_arrow,
                ShapeKind::Line => loc.overlay_studio_badge_line,
            },
            t.accent_ink,
        ),
        OverlayElement::Graph { .. } => (loc.overlay_studio_badge_graph, t.good),
        OverlayElement::ClockTimer { .. } => (loc.overlay_studio_badge_clock, t.ink),
        OverlayElement::InteractiveControl { .. } => (loc.overlay_studio_badge_control, t.accent_ink),
        OverlayElement::SystemTelemetry { .. } => (loc.overlay_studio_badge_status, t.warn),
        OverlayElement::MediaWidget { .. } => (loc.overlay_studio_badge_media, t.good),
        OverlayElement::KeyVisualizer { .. } => (loc.overlay_studio_badge_keys, t.faint),
    }
}

/// Canvas-space centre of one element. Crosshair `x`/`y` are absolute offsets from the card
/// corner and treat 0 as the "centre of the card" sentinel; every other kind is laid out from
/// its top-left corner. Shared by the hit-test, the selection handles and the selection
/// highlight so the copies cannot drift apart (U02-013).
fn element_center_on_canvas(card_canvas_rect: Rect, elem: &OverlayElement, zoom: f32) -> Pos2 {
    match elem {
        OverlayElement::Crosshair { x, y, .. } => {
            if *x > 0.0 && *y > 0.0 {
                card_canvas_rect.min + vec2(*x * zoom, *y * zoom)
            } else {
                card_canvas_rect.center()
            }
        }
        _ => {
            let (raw_w, raw_h) = get_element_size(elem);
            card_canvas_rect.min + vec2((elem.x() + raw_w * 0.5) * zoom, (elem.y() + raw_h * 0.5) * zoom)
        }
    }
}

/// Selection handles of the currently selected element, in canvas coordinates.
#[derive(Debug, Clone, Copy)]
struct SelectionHandles {
    idx: usize,
    rot_rect: Rect,
    resize_rect: Rect,
    center: Pos2,
    half_w: f32,
    half_h: f32,
}

/// What the pointer is over when a canvas drag starts. Resolved once so the selected and the
/// no-selection paths share one hit order instead of two copies (U02-013).
#[derive(Debug, Clone, Copy, PartialEq)]
enum CanvasHit {
    Rotate {
        idx: usize,
        center: Pos2,
        base_angle_rad: f32,
    },
    Resize {
        idx: usize,
    },
    Element {
        idx: usize,
    },
    CardResize,
    CardBody,
    Empty,
}

/// Hit order: rotation and resize handles of the selected element, then elements (topmost
/// first), then the card resize handle, then the card body/pill, then empty canvas.
fn canvas_hit_test(
    pos: Pos2,
    selected_handles: Option<SelectionHandles>,
    elem_rects: &[(usize, Rect)],
    card_resize_handle: Rect,
    pill_rect: Rect,
    card_canvas_rect: Rect,
) -> CanvasHit {
    if let Some(handles) = selected_handles {
        if handles.rot_rect.expand(4.0).contains(pos) {
            let unrot_vec = vec2(-handles.half_w - 3.0, -handles.half_h - 3.0);
            return CanvasHit::Rotate {
                idx: handles.idx,
                center: handles.center,
                base_angle_rad: unrot_vec.y.atan2(unrot_vec.x),
            };
        }
        if handles.resize_rect.expand(4.0).contains(pos) {
            return CanvasHit::Resize { idx: handles.idx };
        }
    }
    if let Some((hit_idx, _)) = elem_rects.iter().rev().find(|(_, r)| r.expand(3.0).contains(pos)) {
        return CanvasHit::Element { idx: *hit_idx };
    }
    if card_resize_handle.expand(4.0).contains(pos) {
        return CanvasHit::CardResize;
    }
    if pill_rect.expand(2.0).contains(pos) || card_canvas_rect.contains(pos) {
        return CanvasHit::CardBody;
    }
    CanvasHit::Empty
}

/// Applies the side effects of starting a canvas drag (undo snapshot before the first mutation,
/// selection change) and returns the drag mode to track. One implementation for every hit path
/// (U02-013).
fn begin_canvas_drag(
    studio: &mut OverlayStudioState,
    hit: CanvasHit,
    pos: Pos2,
    card_sx: f32,
    card_sy: f32,
) -> CanvasDragMode {
    match hit {
        CanvasHit::Rotate { idx, center, base_angle_rad } => {
            studio.push_undo_snapshot();
            CanvasDragMode::Rotate { idx, center, base_angle_rad }
        }
        CanvasHit::Resize { idx } => {
            studio.push_undo_snapshot();
            let (raw_w, raw_h) = match studio.overlay.elements.get(idx) {
                Some(OverlayElement::Crosshair { size, .. }) => (*size, *size),
                Some(OverlayElement::Text { font_size, .. }) => (*font_size, *font_size),
                Some(elem) => get_element_size(elem),
                None => (0.0, 0.0),
            };
            CanvasDragMode::Resize {
                idx,
                start_w: raw_w,
                start_h: raw_h,
                start_pointer: pos,
            }
        }
        CanvasHit::Element { idx } => {
            studio.push_undo_snapshot();
            studio.selected_element_idx = Some(idx);
            let (elem_x, elem_y) = match studio.overlay.elements.get(idx) {
                Some(elem) => (elem.x(), elem.y()),
                None => (0.0, 0.0),
            };
            CanvasDragMode::Move {
                idx,
                start_x: elem_x,
                start_y: elem_y,
                start_pointer: pos,
            }
        }
        CanvasHit::CardResize => {
            studio.push_undo_snapshot();
            studio.selected_element_idx = None;
            CanvasDragMode::ResizeCard {
                start_pointer: pos,
                start_w: studio.overlay.rect.width,
                start_h: studio.overlay.rect.height,
            }
        }
        CanvasHit::CardBody => {
            studio.push_undo_snapshot();
            studio.selected_element_idx = None;
            CanvasDragMode::MoveCard {
                start_pointer: pos,
                start_card_sx: card_sx,
                start_card_sy: card_sy,
            }
        }
        CanvasHit::Empty => {
            studio.selected_element_idx = None;
            CanvasDragMode::Pan { start_pointer: pos, start_pan: studio.canvas_pan }
        }
    }
}

fn render_overlay_canvas(
    ui: &mut Ui,
    studio: &mut OverlayStudioState,
    t: &Tokens,
    loc: &crate::i18n::Strings,
) {
    let monitors = crate::daemon::overlay_manager::get_system_monitors();
    let active_mon = resolve_overlay_monitor(&studio.overlay, &monitors);
    let screen_w = active_mon.width as f32;
    let screen_h = active_mon.height as f32;
    clamp_card_to_monitor(&mut studio.overlay, screen_w, screen_h);

    let min_x = monitors.iter().map(|m| m.x).min().unwrap_or(0);
    let min_y = monitors.iter().map(|m| m.y).min().unwrap_or(0);
    let max_x = monitors.iter().map(|m| m.x + m.width as i32).max().unwrap_or(1920);
    let max_y = monitors.iter().map(|m| m.y + m.height as i32).max().unwrap_or(1080);
    let desktop_w = (max_x - min_x).max(1) as f32;
    let desktop_h = (max_y - min_y).max(1) as f32;
    let desktop_center_x = (min_x + max_x) as f32 * 0.5;
    let desktop_center_y = (min_y + max_y) as f32 * 0.5;

    let avail_canvas_w = ui.available_width().max(80.0);
    let avail_canvas_h = (ui.available_height() - 44.0).max(80.0);

    // 1. Calculate default auto-fit zoom for all monitors on the virtual desktop
    let fit_margin = 32.0;
    let fit_scale_x = (avail_canvas_w - fit_margin * 2.0).max(40.0) / desktop_w;
    let fit_scale_y = (avail_canvas_h - fit_margin * 2.0).max(40.0) / desktop_h;
    let auto_zoom = fit_scale_x.min(fit_scale_y).clamp(0.05, 5.0);

    let cur_zoom = studio.canvas_zoom.unwrap_or(auto_zoom);

    // Full-width Header Toolbar with undo/redo, camera controls, and snap toggle
    // Aligned on the right with the canvas below
    let tb_frame = egui::Frame::new()
        .fill(t.sunken)
        .stroke(t.hairline())
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(8, 4));
    let tb_content_w = (avail_canvas_w - tb_frame.total_margin().sum().x).max(60.0);

    tb_frame.show(ui, |ui| {
        ui.set_width(tb_content_w);
        ui.horizontal(|ui| {
                // 1. LEFT: Undo & Redo buttons
                // Undo Button (ArrowUUpLeft)
                let can_undo = studio.can_undo();
                let undo_color = if can_undo { t.ink } else { t.faint.gamma_multiply(0.35) };
                let (undo_rect, undo_resp) = ui.allocate_exact_size(vec2(18.0, 18.0), if can_undo { Sense::click() } else { Sense::hover() });
                if can_undo && undo_resp.hovered() {
                    ui.painter().rect_filled(undo_rect, CornerRadius::same(2), t.bg);
                }
                ui.painter().text(
                    undo_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    egui_phosphor::regular::ARROW_U_UP_LEFT,
                    FontId::new(12.0, theme::phosphor()),
                    undo_color,
                );
                if can_undo && undo_resp.on_hover_text(loc.overlay_studio_undo_tooltip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    studio.undo();
                }

                // Redo Button (ArrowUUpRight)
                let can_redo = studio.can_redo();
                let redo_color = if can_redo { t.ink } else { t.faint.gamma_multiply(0.35) };
                let (redo_rect, redo_resp) = ui.allocate_exact_size(vec2(18.0, 18.0), if can_redo { Sense::click() } else { Sense::hover() });
                if can_redo && redo_resp.hovered() {
                    ui.painter().rect_filled(redo_rect, CornerRadius::same(2), t.bg);
                }
                ui.painter().text(
                    redo_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    egui_phosphor::regular::ARROW_U_UP_RIGHT,
                    FontId::new(12.0, theme::phosphor()),
                    redo_color,
                );
                if can_redo && redo_resp.on_hover_text(loc.overlay_studio_redo_tooltip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    studio.redo();
                }

                // 2. RIGHT: Camera controls and snap toggle (right-to-left layout)
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;

                    // 7. Zoom In (+) Button (Rightmost)
                    let (in_rect, in_resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    if in_resp.hovered() {
                        ui.painter().rect_filled(in_rect, CornerRadius::same(2), t.bg);
                    }
                    ui.painter().text(
                        in_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        egui_phosphor::regular::PLUS,
                        FontId::new(12.0, theme::phosphor()),
                        t.faint,
                    );
                    if in_resp.on_hover_text(loc.overlay_studio_zoom_in).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        studio.canvas_zoom = Some((cur_zoom * 1.2).clamp(0.08, 10.0));
                    }

                    // 6. Zoom Level Display / Reset to 100%
                    let zoom_pct = (cur_zoom * 100.0).round() as i32;
                    let zoom_str = format!("{zoom_pct}%");
                    let z_text_w = ui.painter().layout_no_wrap(zoom_str.clone(), FontId::new(10.5, theme::mono_bold()), t.ink).size().x + 8.0;
                    let (z_rect, z_resp) = ui.allocate_exact_size(vec2(z_text_w, 18.0), Sense::click());
                    if z_resp.hovered() {
                        ui.painter().rect_filled(z_rect, CornerRadius::same(2), t.bg);
                    }
                    ui.painter().text(
                        z_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        &zoom_str,
                        FontId::new(10.5, theme::mono_bold()),
                        t.accent_ink,
                    );
                    if z_resp.on_hover_text(loc.overlay_studio_reset_zoom).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        studio.canvas_zoom = Some(1.0);
                        studio.canvas_pan = Vec2::ZERO;
                    }

                    // 5. Zoom Out (-) Button
                    let (out_rect, out_resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    if out_resp.hovered() {
                        ui.painter().rect_filled(out_rect, CornerRadius::same(2), t.bg);
                    }
                    ui.painter().text(
                        out_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        egui_phosphor::regular::MINUS,
                        FontId::new(12.0, theme::phosphor()),
                        t.faint,
                    );
                    if out_resp.on_hover_text(loc.overlay_studio_zoom_out).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        studio.canvas_zoom = Some((cur_zoom / 1.2).clamp(0.08, 10.0));
                    }

                    // Divider
                    let (div_rect2, _) = ui.allocate_exact_size(vec2(1.0, 14.0), Sense::hover());
                    ui.painter().vline(div_rect2.center().x, div_rect2.y_range(), t.hairline());

                    // 4. Fit Screen Button
                    let (fit_rect, fit_resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    if fit_resp.hovered() {
                        ui.painter().rect_filled(fit_rect, CornerRadius::same(2), t.bg);
                    }
                    ui.painter().text(
                        fit_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        egui_phosphor::regular::ARROWS_OUT,
                        FontId::new(12.0, theme::phosphor()),
                        t.faint,
                    );
                    if fit_resp.on_hover_text(loc.overlay_studio_fit_screen).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        studio.canvas_pan = Vec2::ZERO;
                        studio.canvas_zoom = Some(auto_zoom);
                    }

                    // 3. Focus Overlay Card Button
                    let (focus_rect, focus_resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    if focus_resp.hovered() {
                        ui.painter().rect_filled(focus_rect, CornerRadius::same(2), t.bg);
                    }
                    ui.painter().text(
                        focus_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        egui_phosphor::regular::TARGET,
                        FontId::new(12.0, theme::phosphor()),
                        t.faint,
                    );
                    if focus_resp.on_hover_text(loc.overlay_studio_focus_card).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        let (csx, csy) = compute_overlay_screen_pos(&studio.overlay, screen_w, screen_h);
                        let card_center_vx = active_mon.x as f32 + csx + studio.overlay.rect.width * 0.5;
                        let card_center_vy = active_mon.y as f32 + csy + studio.overlay.rect.height * 0.5;
                        let focus_margin = 60.0;
                        let f_zoom_x = (avail_canvas_w - focus_margin * 2.0).max(40.0) / studio.overlay.rect.width.max(32.0);
                        let f_zoom_y = (avail_canvas_h - focus_margin * 2.0).max(40.0) / studio.overlay.rect.height.max(32.0);
                        let target_zoom = f_zoom_x.min(f_zoom_y).clamp(0.2, 5.0);
                        studio.canvas_pan = -vec2((card_center_vx - desktop_center_x) * target_zoom, (card_center_vy - desktop_center_y) * target_zoom);
                        studio.canvas_zoom = Some(target_zoom);
                    }

                    // 2. Snap to Grid Toggle
                    let grid_glyph = egui_phosphor::regular::MAGNET;
                    let grid_color = if studio.snap_to_grid { t.accent_ink } else { t.faint };
                    let (grid_rect, grid_resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    if studio.snap_to_grid {
                        ui.painter().rect_filled(grid_rect, CornerRadius::same(2), t.accent_ink.gamma_multiply(0.2));
                    } else if grid_resp.hovered() {
                        ui.painter().rect_filled(grid_rect, CornerRadius::same(2), t.bg);
                    }
                    ui.painter().text(
                        grid_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        grid_glyph,
                        FontId::new(12.0, theme::phosphor()),
                        grid_color,
                    );
                    if grid_resp.on_hover_text(loc.overlay_studio_snap_grid).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        studio.snap_to_grid = !studio.snap_to_grid;
                    }
                });
            });
        });
    ui.add_space(4.0);

    let canvas_size = vec2(
        avail_canvas_w,
        (ui.available_height() - 6.0).max(80.0),
    );
    let (canvas_rect, _resp) = ui.allocate_exact_size(canvas_size, Sense::hover());
    let painter = ui.painter().with_clip_rect(canvas_rect);

    let zoom = studio.canvas_zoom.unwrap_or(auto_zoom);
    let pan = studio.canvas_pan;

    let canvas_center = canvas_rect.center() + pan;
    let virtual_to_canvas = |vx: f32, vy: f32| -> Pos2 {
        canvas_center + vec2((vx - desktop_center_x) * zoom, (vy - desktop_center_y) * zoom)
    };
    let canvas_to_virtual = |cp: Pos2| -> (f32, f32) {
        let offset = cp - canvas_center;
        (desktop_center_x + offset.x / zoom, desktop_center_y + offset.y / zoom)
    };

    let monitor_rects: Vec<(usize, Rect)> = monitors.iter().map(|m| {
        let m_min = virtual_to_canvas(m.x as f32, m.y as f32);
        let m_size = vec2(m.width as f32 * zoom, m.height as f32 * zoom);
        (m.index, Rect::from_min_size(m_min, m_size))
    }).collect();

    // 2. Draw workspace canvas background (Infinite artboard background)
    painter.rect_filled(canvas_rect, CornerRadius::same(6), Color32::from_rgb(11, 12, 15));
    painter.rect_stroke(canvas_rect, CornerRadius::same(6), t.hairline(), egui::StrokeKind::Inside);

    // Workspace dot grid outside monitors
    let dot_step = 24.0;
    let mut gx = canvas_rect.left() + dot_step;
    while gx < canvas_rect.right() {
        let mut gy = canvas_rect.top() + dot_step;
        while gy < canvas_rect.bottom() {
            let pt = pos2(gx, gy);
            if !monitor_rects.iter().any(|(_, mr)| mr.contains(pt)) {
                painter.circle_filled(pt, 1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 12));
            }
            gy += dot_step;
        }
        gx += dot_step;
    }

    // 3. Draw Monitor Artboards in true spatial layout
    for m in &monitors {
        let is_active = m.index == active_mon.index;
        let m_rect = monitor_rects.iter().find(|(idx, _)| *idx == m.index).map(|(_, r)| *r).unwrap_or(Rect::ZERO);

        let mon_w = m.width.to_string();
        let mon_h = m.height.to_string();
        let mon_label = fill(loc.overlay_studio_canvas_display, &[&mon_w, &mon_h]);

        if is_active {
            // Active Monitor Artboard (illuminated border + subtle surface)
            painter.rect_filled(m_rect.expand(4.0), CornerRadius::same(8), Color32::from_rgba_unmultiplied(0, 0, 0, 110));
            painter.rect_filled(m_rect, CornerRadius::same(6), Color32::from_rgb(17, 19, 24));
            painter.rect_stroke(
                m_rect,
                CornerRadius::same(6),
                Stroke::new(1.5, t.accent_ink),
                egui::StrokeKind::Inside,
            );

            // Active monitor resolution badge
            let mon_font = FontId::new((10.0f32 * zoom).clamp(8.0, 12.0), theme::mono_bold());
            let label_pos = m_rect.min + vec2(8.0, 6.0);
            if m_rect.height() > 24.0 {
                painter.text(label_pos, egui::Align2::LEFT_TOP, mon_label, mon_font, t.accent_ink);
            }

            // Monitor center crosslines (subtle guide)
            let m_center = m_rect.center();
            let center_stroke = if studio.snap_to_grid {
                Stroke::new(1.0, t.accent_ink.gamma_multiply(0.35))
            } else {
                Stroke::new(0.8, Color32::from_rgba_unmultiplied(255, 255, 255, 18))
            };
            painter.vline(m_center.x, m_rect.y_range(), center_stroke);
            painter.hline(m_rect.x_range(), m_center.y, center_stroke);

            // Monitor grid if snap-to-grid is active
            if studio.snap_to_grid {
                let grid_step = 64.0 * zoom;
                if grid_step >= 12.0 {
                    let mut mgx = m_rect.left() + grid_step;
                    while mgx < m_rect.right() - 1.0 {
                        painter.vline(mgx, m_rect.y_range(), Stroke::new(0.5, Color32::from_rgba_unmultiplied(255, 255, 255, 10)));
                        mgx += grid_step;
                    }
                    let mut mgy = m_rect.top() + grid_step;
                    while mgy < m_rect.bottom() - 1.0 {
                        painter.hline(m_rect.x_range(), mgy, Stroke::new(0.5, Color32::from_rgba_unmultiplied(255, 255, 255, 10)));
                        mgy += grid_step;
                    }
                }
            }
        } else {
            // Inactive Monitor Artboard (dimmed surface + subtle border)
            painter.rect_filled(m_rect.expand(4.0), CornerRadius::same(8), Color32::from_rgba_unmultiplied(0, 0, 0, 80));
            painter.rect_filled(m_rect, CornerRadius::same(6), Color32::from_rgb(13, 14, 18));
            painter.rect_stroke(
                m_rect,
                CornerRadius::same(6),
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 24)),
                egui::StrokeKind::Inside,
            );

            let mon_font = FontId::new((10.0f32 * zoom).clamp(8.0, 11.0), theme::mono());
            let label_pos = m_rect.min + vec2(8.0, 6.0);
            if m_rect.height() > 24.0 {
                painter.text(label_pos, egui::Align2::LEFT_TOP, mon_label, mon_font, Color32::from_rgba_unmultiplied(255, 255, 255, 55));
            }
        }
    }

    // 4. Compute Card position and geometry on canvas
    let (card_sx, card_sy) = compute_overlay_screen_pos(&studio.overlay, screen_w, screen_h);
    let card_virtual_min = pos2(active_mon.x as f32 + card_sx, active_mon.y as f32 + card_sy);
    let card_canvas_min = virtual_to_canvas(card_virtual_min.x, card_virtual_min.y);
    let card_canvas_size = vec2(studio.overlay.rect.width * zoom, studio.overlay.rect.height * zoom);
    let card_canvas_rect = Rect::from_min_size(card_canvas_min, card_canvas_size);

    let pill_h = 16.0;
    let pill_w = 114.0;
    let pill_top = (card_canvas_rect.top() - pill_h - 3.0).max(canvas_rect.top() + 4.0);
    let pill_rect = Rect::from_min_size(pos2(card_canvas_rect.left(), pill_top), vec2(pill_w, pill_h));

    let card_resize_handle = Rect::from_center_size(card_canvas_rect.max, vec2(14.0, 14.0));

    // Global Opacity Helper
    let global_opacity = studio.overlay.opacity.clamp(0.0, 1.0);
    let apply_opacity = move |rgba: [u8; 4]| -> Color32 {
        let a = ((rgba[3] as f32) * global_opacity).round() as u8;
        Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], a)
    };

    // Calculate bounding boxes for all elements on the canvas (using rotated AABB)
    let elem_rects: Vec<(usize, Rect)> = studio.overlay.elements.iter().enumerate().map(|(i, elem)| {
        let (raw_w, raw_h) = get_element_size(elem);
        let ew = (raw_w * zoom).max(4.0);
        let eh = (raw_h * zoom).max(4.0);
        let angle_rad = elem.rotation().to_radians();
        let center = element_center_on_canvas(card_canvas_rect, elem, zoom);
        (i, get_rotated_aabb(center, ew * 0.5, eh * 0.5, angle_rad))
    }).collect();

    let drag_mode_id = ui.scope_id().with("canvas_drag_mode");
    let mut drag_mode: CanvasDragMode = ui.ctx().data(|d| d.get_temp(drag_mode_id)).unwrap_or(CanvasDragMode::None);

    let selected_handles = studio.selected_element_idx.and_then(|sel| {
        if sel < studio.overlay.elements.len() {
            let elem = &studio.overlay.elements[sel];
            let (raw_w, raw_h) = get_element_size(elem);
            let ew = (raw_w * zoom).max(4.0);
            let eh = (raw_h * zoom).max(4.0);
            let angle_rad = elem.rotation().to_radians();
            let center = element_center_on_canvas(card_canvas_rect, elem, zoom);
            let half_w = ew * 0.5;
            let half_h = eh * 0.5;
            let p0 = rotate_point(center + vec2(-half_w - 3.0, -half_h - 3.0), center, angle_rad);
            let p2 = rotate_point(center + vec2(half_w + 3.0, half_h + 3.0), center, angle_rad);
            let rot_rect = Rect::from_center_size(p0, vec2(16.0, 16.0));
            let resize_rect = Rect::from_center_size(p2, vec2(16.0, 16.0));
            Some(SelectionHandles {
                idx: sel,
                rot_rect,
                resize_rect,
                center,
                half_w,
                half_h,
            })
        } else {
            None
        }
    });

    // ---------------- CANVAS DIRECT MOUSE DRAG & SELECTION & RESIZING / ROTATING ----------------
    let canvas_sense_resp = ui.interact(canvas_rect, ui.scope_id().with("canvas_mouse_sense"), Sense::click_and_drag());
    let pointer_pos = ui.input(|i| i.pointer.hover_pos().or(i.pointer.interact_pos()));

    let is_space_down = ui.input(|i| i.key_down(egui::Key::Space));
    let is_middle_down = ui.input(|i| i.pointer.middle_down());
    let is_secondary_down = ui.input(|i| i.pointer.secondary_down());

    if canvas_sense_resp.drag_started() {
        if let Some(pos) = pointer_pos {
            if is_space_down || is_middle_down || is_secondary_down {
                drag_mode = CanvasDragMode::Pan { start_pointer: pos, start_pan: studio.canvas_pan };
            } else {
                let hit = canvas_hit_test(
                    pos,
                    selected_handles,
                    &elem_rects,
                    card_resize_handle,
                    pill_rect,
                    card_canvas_rect,
                );
                drag_mode = begin_canvas_drag(studio, hit, pos, card_sx, card_sy);
            }
        }
    } else if canvas_sense_resp.clicked() {
        if let Some(pos) = pointer_pos {
            let hit = elem_rects.iter().rev().find(|(_, r)| r.expand(3.0).contains(pos));
            if let Some((i, _)) = hit {
                let hit_idx = *i;
                studio.selected_element_idx = Some(hit_idx);
                // Explicit studio-local testing: the preview does not forward pointer input to
                // the renderer (see `studio_preview_render_inputs`), so stateful widgets are
                // exercised on the real element here (U02-002).
                studio_test_click(&mut studio.overlay.elements[hit_idx]);
            } else if !card_canvas_rect.contains(pos) && !pill_rect.contains(pos) {
                studio.selected_element_idx = None;
                for m in &monitors {
                    if m.index != active_mon.index {
                        if let Some((_, mr)) = monitor_rects.iter().find(|(idx, _)| *idx == m.index) {
                            if mr.contains(pos) {
                                studio.overlay.monitor_index = Some(m.index);
                                clamp_card_to_monitor(&mut studio.overlay, m.width as f32, m.height as f32);
                                clamp_elements_to_canvas(&mut studio.overlay);
                                break;
                            }
                        }
                    }
                }
            } else {
                studio.selected_element_idx = None;
            }
        }
    }

    let snap = |val: f32| -> f32 {
        if studio.snap_to_grid {
            (val / 16.0).round() * 16.0
        } else {
            val.round()
        }
    };

    if canvas_sense_resp.dragged() {
        if let Some(pos) = pointer_pos {
            match &mut drag_mode {
                CanvasDragMode::Pan { start_pointer, start_pan } => {
                    studio.canvas_pan = *start_pan + (pos - *start_pointer);
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                }
                CanvasDragMode::MoveCard { start_pointer, start_card_sx, start_card_sy } => {
                    // Safeguard: only auto-pan camera across canvas when the overlay card itself is selected (no element selected)
                    if studio.selected_element_idx.is_none() {
                        let edge_zone = 70.0;
                        let (cur_rx, cur_ry) = compute_overlay_screen_pos(&studio.overlay, screen_w, screen_h);
                        let max_card_x = (screen_w - studio.overlay.rect.width).max(0.0);
                        let max_card_y = (screen_h - studio.overlay.rect.height).max(0.0);

                        let can_pan_right = cur_rx < max_card_x - 0.5;
                        let can_pan_left = cur_rx > 0.5;
                        let can_pan_down = cur_ry < max_card_y - 0.5;
                        let can_pan_up = cur_ry > 0.5;

                        let mut pan_step = Vec2::ZERO;

                        if can_pan_right && pos.x > canvas_rect.right() - edge_zone {
                            let factor = ((pos.x - (canvas_rect.right() - edge_zone)) / edge_zone).clamp(0.0, 1.5);
                            pan_step.x -= 3.0 + 8.0 * factor;
                        } else if can_pan_left && pos.x < canvas_rect.left() + edge_zone {
                            let factor = ((canvas_rect.left() + edge_zone - pos.x) / edge_zone).clamp(0.0, 1.5);
                            pan_step.x += 3.0 + 8.0 * factor;
                        }

                        if can_pan_down && pos.y > canvas_rect.bottom() - edge_zone {
                            let factor = ((pos.y - (canvas_rect.bottom() - edge_zone)) / edge_zone).clamp(0.0, 1.5);
                            pan_step.y -= 3.0 + 8.0 * factor;
                        } else if can_pan_up && pos.y < canvas_rect.top() + edge_zone {
                            let factor = ((canvas_rect.top() + edge_zone - pos.y) / edge_zone).clamp(0.0, 1.5);
                            pan_step.y += 3.0 + 8.0 * factor;
                        }

                        if pan_step != Vec2::ZERO {
                            studio.canvas_pan += pan_step;
                            *start_pointer += pan_step;
                            ui.ctx().request_repaint();
                        }
                    }

                    let delta = (pos - *start_pointer) / zoom.max(0.05);
                    let mut new_rx = snap(*start_card_sx + delta.x);
                    let mut new_ry = snap(*start_card_sy + delta.y);

                    // Magnetic monitor-center snap guide (within 12px)
                    let center_x = (screen_w - studio.overlay.rect.width) * 0.5;
                    let center_y = (screen_h - studio.overlay.rect.height) * 0.5;
                    let is_center_x = (new_rx - center_x).abs() < 12.0;
                    let is_center_y = (new_ry - center_y).abs() < 12.0;
                    if is_center_x { new_rx = center_x; }
                    if is_center_y { new_ry = center_y; }

                    new_rx = new_rx.clamp(0.0, (screen_w - studio.overlay.rect.width).max(0.0));
                    new_ry = new_ry.clamp(0.0, (screen_h - studio.overlay.rect.height).max(0.0));

                    if is_center_x && is_center_y {
                        studio.overlay.anchor = OverlayAnchor::ScreenCenter;
                        studio.overlay.rect.x = 0.0;
                        studio.overlay.rect.y = 0.0;
                    } else {
                        studio.overlay.anchor = OverlayAnchor::Custom;
                        studio.overlay.rect.x = new_rx;
                        studio.overlay.rect.y = new_ry;
                    }
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                }
                CanvasDragMode::ResizeCard { start_pointer, start_w, start_h } => {
                    // Safeguard: only auto-pan camera across canvas when no element is selected
                    if studio.selected_element_idx.is_none() {
                        let edge_zone = 70.0;
                        let (cur_rx, cur_ry) = compute_overlay_screen_pos(&studio.overlay, screen_w, screen_h);
                        let can_pan_right = cur_rx + studio.overlay.rect.width < screen_w - 0.5;
                        let can_pan_down = cur_ry + studio.overlay.rect.height < screen_h - 0.5;

                        let mut pan_step = Vec2::ZERO;
                        if can_pan_right && pos.x > canvas_rect.right() - edge_zone {
                            let factor = ((pos.x - (canvas_rect.right() - edge_zone)) / edge_zone).clamp(0.0, 1.5);
                            pan_step.x -= 3.0 + 8.0 * factor;
                        }
                        if can_pan_down && pos.y > canvas_rect.bottom() - edge_zone {
                            let factor = ((pos.y - (canvas_rect.bottom() - edge_zone)) / edge_zone).clamp(0.0, 1.5);
                            pan_step.y -= 3.0 + 8.0 * factor;
                        }
                        if pan_step != Vec2::ZERO {
                            studio.canvas_pan += pan_step;
                            *start_pointer += pan_step;
                            ui.ctx().request_repaint();
                        }
                    }

                    let (max_w, max_h) = max_card_dimensions(&studio.overlay, screen_w, screen_h);
                    let delta = (pos - *start_pointer) / zoom.max(0.05);
                    let is_locked = studio.card_lock_aspect || ui.input(|i| i.modifiers.shift);
                    if is_locked {
                        let aspect = (*start_w / (*start_h).max(1.0)).clamp(0.05, 50.0);
                        let d = if delta.x.abs() > delta.y.abs() { delta.x } else { delta.y * aspect };
                        let effective_max_w = max_w.min(max_h * aspect);
                        let effective_max_h = max_h.min(max_w / aspect.max(0.01));
                        let new_w = snap(*start_w + d).clamp(20.0, effective_max_w);
                        let new_h = (new_w / aspect).round().clamp(20.0, effective_max_h);
                        studio.overlay.rect.width = new_w;
                        studio.overlay.rect.height = new_h;
                    } else {
                        studio.overlay.rect.width = snap(*start_w + delta.x).clamp(20.0, max_w);
                        studio.overlay.rect.height = snap(*start_h + delta.y).clamp(20.0, max_h);
                    }
                    clamp_elements_to_canvas(&mut studio.overlay);
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
                }
                CanvasDragMode::Move { idx, start_x, start_y, start_pointer } => {
                    if *idx < studio.overlay.elements.len() {
                        let total_delta = (pos - *start_pointer) / zoom.max(0.05);
                        let (ew, eh) = get_element_size(&studio.overlay.elements[*idx]);
                        match &mut studio.overlay.elements[*idx] {
                            OverlayElement::Text { x, y, .. }
                            | OverlayElement::Image { x, y, .. }
                            | OverlayElement::Shape { x, y, .. }
                            | OverlayElement::Graph { x, y, .. }
                            | OverlayElement::ClockTimer { x, y, .. }
                            | OverlayElement::InteractiveControl { x, y, .. }
                            | OverlayElement::SystemTelemetry { x, y, .. }
                            | OverlayElement::MediaWidget { x, y, .. }
                            | OverlayElement::KeyVisualizer { x, y, .. } => {
                                *x = snap(*start_x + total_delta.x).clamp(0.0, (studio.overlay.rect.width - ew).max(0.0));
                                *y = snap(*start_y + total_delta.y).clamp(0.0, (studio.overlay.rect.height - eh).max(0.0));
                            }
                            OverlayElement::Crosshair { x, y, .. } => {
                                *x = clamp_crosshair_coord(snap(*start_x + total_delta.x), studio.overlay.rect.width);
                                *y = clamp_crosshair_coord(snap(*start_y + total_delta.y), studio.overlay.rect.height);
                            }
                        }
                        clamp_elements_to_canvas(&mut studio.overlay);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                }
                CanvasDragMode::Rotate { idx, center, base_angle_rad } => {
                    if *idx < studio.overlay.elements.len() {
                        let v = pos - *center;
                        if v.length_sq() > 4.0 {
                            let current_angle = v.y.atan2(v.x);
                            let diff_rad = current_angle - *base_angle_rad;
                            let mut deg = diff_rad.to_degrees().rem_euclid(360.0);
                            if ui.input(|i| i.modifiers.shift) {
                                deg = (deg / 15.0).round() * 15.0;
                                deg = deg.rem_euclid(360.0);
                            } else {
                                deg = deg.round();
                            }
                            *studio.overlay.elements[*idx].rotation_mut() = deg;
                        }
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                    }
                }
                CanvasDragMode::Resize { idx, start_w, start_h, start_pointer } => {
                    if *idx < studio.overlay.elements.len() {
                        let total_delta = (pos - *start_pointer) / zoom.max(0.05);
                        match &mut studio.overlay.elements[*idx] {
                            OverlayElement::Text { font_size, .. } => {
                                let delta = (total_delta.x + total_delta.y) * 0.25;
                                *font_size = snap(*start_w + delta).clamp(6.0, 72.0);
                            }
                            OverlayElement::Image { width, height, lock_aspect, x, y, .. } => {
                                let max_w = (studio.overlay.rect.width - *x).max(8.0);
                                let max_h = (studio.overlay.rect.height - *y).max(8.0);
                                let is_locked = *lock_aspect || ui.input(|i| i.modifiers.shift);
                                if is_locked {
                                    let ratio = (*start_w / (*start_h).max(1.0)).clamp(0.05, 50.0);
                                    let delta = if total_delta.x.abs() > total_delta.y.abs() {
                                        total_delta.x
                                    } else {
                                        total_delta.y * ratio
                                    };
                                    let new_w = snap(*start_w + delta).clamp(8.0, max_w.min(max_h * ratio));
                                    *width = new_w;
                                    *height = (new_w / ratio).round().clamp(8.0, max_h);
                                } else {
                                    *width = snap(*start_w + total_delta.x).clamp(8.0, max_w);
                                    *height = snap(*start_h + total_delta.y).clamp(8.0, max_h);
                                }
                            }
                            OverlayElement::Shape { width, height, lock_aspect, x, y, .. } => {
                                let max_w = (studio.overlay.rect.width - *x).max(8.0);
                                let max_h = (studio.overlay.rect.height - *y).max(8.0);
                                let is_locked = *lock_aspect || ui.input(|i| i.modifiers.shift);
                                if is_locked {
                                    let ratio = (*start_w / (*start_h).max(1.0)).clamp(0.05, 50.0);
                                    let delta = if total_delta.x.abs() > total_delta.y.abs() {
                                        total_delta.x
                                    } else {
                                        total_delta.y * ratio
                                    };
                                    let new_w = snap(*start_w + delta).clamp(8.0, max_w.min(max_h * ratio));
                                    *width = new_w;
                                    *height = (new_w / ratio).round().clamp(8.0, max_h);
                                } else {
                                    *width = snap(*start_w + total_delta.x).clamp(8.0, max_w);
                                    *height = snap(*start_h + total_delta.y).clamp(8.0, max_h);
                                }
                            }
                            OverlayElement::Graph { width, height, x, y, .. }
                            | OverlayElement::ClockTimer { width, height, x, y, .. }
                            | OverlayElement::InteractiveControl { width, height, x, y, .. }
                            | OverlayElement::SystemTelemetry { width, height, x, y, .. }
                            | OverlayElement::MediaWidget { width, height, x, y, .. }
                            | OverlayElement::KeyVisualizer { width, height, x, y, .. } => {
                                let max_w = (studio.overlay.rect.width - *x).max(10.0);
                                let max_h = (studio.overlay.rect.height - *y).max(10.0);
                                *width = snap(*start_w + total_delta.x).clamp(10.0, max_w);
                                *height = snap(*start_h + total_delta.y).clamp(10.0, max_h);
                            }
                            OverlayElement::Crosshair { size, x, y, .. } => {
                                let max_s = (studio.overlay.rect.width - *x).min(studio.overlay.rect.height - *y).max(8.0);
                                *size = snap(*start_w + total_delta.x * 0.5).clamp(4.0, max_s.min(128.0));
                            }
                        }
                        clamp_elements_to_canvas(&mut studio.overlay);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
                    }
                }
                CanvasDragMode::None => {}
            }
        }
    } else {
        drag_mode = CanvasDragMode::None;
        if let Some(pos) = pointer_pos {
            if is_space_down {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            } else if let Some(handles) = selected_handles {
                if handles.rot_rect.expand(3.0).contains(pos) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                } else if handles.resize_rect.expand(3.0).contains(pos) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
                } else if elem_rects.iter().any(|(_, r)| r.expand(3.0).contains(pos)) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                } else if card_resize_handle.expand(3.0).contains(pos) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
                } else if pill_rect.contains(pos) || card_canvas_rect.contains(pos) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            } else if elem_rects.iter().any(|(_, r)| r.expand(3.0).contains(pos)) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            } else if card_resize_handle.expand(3.0).contains(pos) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
            } else if pill_rect.contains(pos) || card_canvas_rect.contains(pos) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
        }
    }

    // Mouse wheel camera zooming (or Alt+wheel for fine element rotation)
    if canvas_sense_resp.hovered() {
        let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll_y.abs() > 0.5 {
            if ui.input(|i| i.modifiers.alt) {
                if let Some(idx) = studio.selected_element_idx {
                    if idx < studio.overlay.elements.len() {
                        let sign = scroll_y.signum();
                        let cur_rot = studio.overlay.elements[idx].rotation();
                        let new_rot = (cur_rot + sign * 5.0).rem_euclid(360.0);
                        *studio.overlay.elements[idx].rotation_mut() = new_rot;
                    }
                }
            } else {
                let zoom_delta = (scroll_y * 0.0016).clamp(-0.075, 0.075);
                let factor = 1.0 + zoom_delta;
                let new_zoom = (zoom * factor).clamp(0.08, 10.0);
                if let Some(pos) = pointer_pos {
                    let (vx, vy) = canvas_to_virtual(pos);
                    let new_center = pos - vec2((vx - desktop_center_x) * new_zoom, (vy - desktop_center_y) * new_zoom);
                    studio.canvas_pan = new_center - canvas_rect.center();
                }
                studio.canvas_zoom = Some(new_zoom);
            }
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(drag_mode_id, drag_mode));

    // 5. Draw Preview Card Background with global opacity applied
    if studio.overlay.style.bg_color[3] > 0 {
        let bg_color = apply_opacity(studio.overlay.style.bg_color);
        let cr = CornerRadius::same((studio.overlay.style.corner_radius * zoom).round() as u8);
        painter.rect_filled(card_canvas_rect, cr, bg_color);
    }

    // Draw Preview Card Border
    let card_border_cr = CornerRadius::same((studio.overlay.style.corner_radius * zoom).round() as u8);
    if studio.overlay.style.border_color[3] > 0 && studio.overlay.style.border_width > 0.0 {
        let border_color = apply_opacity(studio.overlay.style.border_color);
        painter.rect_stroke(
            card_canvas_rect,
            card_border_cr,
            Stroke::new((studio.overlay.style.border_width * zoom).max(1.0), border_color),
            egui::StrokeKind::Inside,
        );
    } else {
        painter.rect_stroke(
            card_canvas_rect,
            card_border_cr,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(100, 160, 255, 60)),
            egui::StrokeKind::Inside,
        );
    }

    // Card Header Pill
    painter.rect_filled(pill_rect, CornerRadius::same(3), Color32::from_rgba_unmultiplied(20, 26, 38, 220));
    painter.rect_stroke(pill_rect, CornerRadius::same(3), Stroke::new(1.0, t.accent_ink.gamma_multiply(0.4)), egui::StrokeKind::Inside);
    let pill_w = (studio.overlay.rect.width as i32).to_string();
    let pill_h = (studio.overlay.rect.height as i32).to_string();
    let pill_text = fill(loc.overlay_studio_canvas_overlay, &[&pill_w, &pill_h]);
    painter.text(pill_rect.center(), egui::Align2::CENTER_CENTER, pill_text, FontId::new(9.5, theme::mono_bold()), t.accent_ink);

    // Card Resize Handle at bottom-right corner
    painter.rect_filled(card_resize_handle, CornerRadius::same(2), t.sunken);
    painter.rect_stroke(card_resize_handle, CornerRadius::same(2), Stroke::new(1.0, t.accent_ink), egui::StrokeKind::Inside);
    painter.line_segment([pos2(card_resize_handle.left() + 3.0, card_resize_handle.bottom() - 3.0), pos2(card_resize_handle.right() - 3.0, card_resize_handle.top() + 3.0)], Stroke::new(1.5, t.accent_ink));

    // 6. Render Overlay Elements inside Card boundaries using Card Clipping
    let card_painter = painter.with_clip_rect(card_canvas_rect.intersect(canvas_rect));
    // The preview deliberately renders the real elements with interaction disabled: a clone
    // would silently discard every renderer-side state change, and live interaction would run
    // the element's real `on_click` from inside the editor (U02-002).
    let preview_inputs = studio_preview_render_inputs();
    let preview_target_process = studio.overlay.target_process.clone();

    for elem_idx in 0..studio.overlay.elements.len() {
        let mut render_ctx = crate::gui::components::overlay_renderer::RenderContext {
            painter: &card_painter,
            card_canvas_rect,
            zoom,
            global_opacity: studio.overlay.opacity,
            is_interactive: preview_inputs.is_interactive,
            pointer_pos,
            pointer_clicked: preview_inputs.pointer_clicked,
            pointer_down: preview_inputs.pointer_down,
            override_text: None,
            target_process: preview_target_process.as_deref(),
        };
        crate::gui::components::overlay_renderer::render_element(
            &mut render_ctx,
            &mut studio.overlay.elements[elem_idx],
        );

        // Draw selection highlight box & interactive resize handle (rotated OBB)
        if studio.selected_element_idx == Some(elem_idx) {
            let elem = &studio.overlay.elements[elem_idx];
            let (raw_w, raw_h) = get_element_size(elem);
            let ew = (raw_w * zoom).max(4.0);
            let eh = (raw_h * zoom).max(4.0);
            let angle_rad = elem.rotation().to_radians();
            let center = element_center_on_canvas(card_canvas_rect, elem, zoom);

            let half_w = ew * 0.5;
            let half_h = eh * 0.5;
            let p0 = rotate_point(center + vec2(-half_w - 3.0, -half_h - 3.0), center, angle_rad);
            let p1 = rotate_point(center + vec2(half_w + 3.0, -half_h - 3.0), center, angle_rad);
            let p2 = rotate_point(center + vec2(half_w + 3.0, half_h + 3.0), center, angle_rad);
            let p3 = rotate_point(center + vec2(-half_w - 3.0, half_h + 3.0), center, angle_rad);

            let stroke = Stroke::new(1.5, t.accent_ink);
            painter.line_segment([p0, p1], stroke);
            painter.line_segment([p1, p2], stroke);
            painter.line_segment([p2, p3], stroke);
            painter.line_segment([p3, p0], stroke);

            // 1. Top-Left: Rotate handle (circular knob)
            painter.circle_filled(p0, 6.0, t.sunken);
            painter.circle_stroke(p0, 6.0, Stroke::new(1.5, t.accent_ink));
            painter.circle_filled(p0, 2.5, t.accent_ink);

            // 2. Bottom-Right: Resize handle (square knob with diagonal line)
            let h_rect = Rect::from_center_size(p2, vec2(12.0, 12.0));
            painter.rect_filled(h_rect, CornerRadius::same(2), t.accent_ink);
            painter.rect_stroke(h_rect, CornerRadius::same(2), Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);
            painter.line_segment([pos2(h_rect.left() + 3.0, h_rect.bottom() - 3.0), pos2(h_rect.right() - 3.0, h_rect.top() + 3.0)], Stroke::new(1.0, Color32::WHITE));

            // 3. Live coordinate tag with element size and rotation, rotating in lockstep with container
            let coord_x = format!("{:.0}", elem.x());
            let coord_y = format!("{:.0}", elem.y());
            let coord_w = format!("{:.0}", raw_w);
            let coord_h = format!("{:.0}", raw_h);
            let coord_rot = format!("{:.0}", elem.rotation());
            let coord_str = fill(
                loc.overlay_studio_canvas_coords,
                &[&coord_x, &coord_y, &coord_w, &coord_h, &coord_rot],
            );
            let font = FontId::new(9.5, theme::mono());
            let galley = painter.layout_no_wrap(coord_str, font, t.accent_ink);
            let tag_local = vec2(-half_w, half_h + 8.0);
            let tag_pos = center + rotate_vec(tag_local, angle_rad);
            let mut text_shape = egui::epaint::TextShape::new(tag_pos, galley, t.accent_ink);
            text_shape.angle = angle_rad;
            painter.add(text_shape);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_overlay_modal_no_crash() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);

        let presets = ["overlay_crosshair", "overlay_macro_badge", "overlay_telemetry", "unknown_custom"];
        for preset in presets {
            let mut state = GuiState::new();
            state.open_overlay_studio(preset);

            for _ in 0..5 {
                let mut out = ctx.run_ui(Default::default(), |ctx| {
                    render_overlay_modal(ctx, &mut state);
                });
                out.textures_delta.clear();
            }

            state.close_overlay_studio();
            for _ in 0..5 {
                let mut out = ctx.run_ui(Default::default(), |ctx| {
                    render_overlay_modal(ctx, &mut state);
                });
                out.textures_delta.clear();
            }
        }
    }

    #[test]
    fn test_overlay_elements_with_fonts_and_coordinates() {
        use crate::models::{OverlayDefinition, OverlayElement};

        let mut overlay = OverlayDefinition::default_crosshair();
        assert_eq!(overlay.elements.len(), 1);
        if let OverlayElement::Crosshair { x, y, .. } = overlay.elements[0] {
            assert_eq!(x, 24.0);
            assert_eq!(y, 24.0);
        } else {
            panic!("Expected crosshair element");
        }

        // Test text font family & click action
        overlay.elements.push(OverlayElement::Text {
            id: "txt_1".to_string(),
            text: "FPS: 144".to_string(),
            font_size: 14.0,
            font_family: "Impact".to_string(),
            bold: false,
            color: [255, 255, 255, 255],
            x: 10.0,
            y: 10.0,
            width: None,
            height: None,
            lock_aspect: true,
            rotation: 0.0,
            on_click: Some(crate::models::OverlayClickAction::CloseOverlay),
        });

        // Test image element
        overlay.elements.push(OverlayElement::Image {
            id: "img_1".to_string(),
            path: "C:/test/sample.png".to_string(),
            x: 5.0,
            y: 5.0,
            width: 32.0,
            height: 32.0,
            opacity: 0.9,
            lock_aspect: true,
            rotation: 0.0,
            on_click: None,
        });

        // Test shape element
        overlay.elements.push(OverlayElement::Shape {
            id: "shape_1".to_string(),
            shape_type: crate::models::ShapeKind::Rectangle,
            x: 20.0,
            y: 20.0,
            width: 40.0,
            height: 40.0,
            thickness: 2.0,
            color: [255, 0, 0, 255],
            filled: true,
            fill_color: [0, 255, 0, 100],
            corner_radius: 6.0,
            lock_aspect: true,
            rotation: 0.0,
            on_click: None,
        });

        assert_eq!(overlay.elements.len(), 4);
    }

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

    #[test]
    fn test_canvas_element_clamping_and_image_fit() {
        use crate::models::{OverlayDefinition, OverlayElement};

        let mut overlay = OverlayDefinition::default_crosshair();
        overlay.rect.width = 100.0;
        overlay.rect.height = 100.0;

        // 1. Proportional Image (lock_aspect: true)
        overlay.elements.push(OverlayElement::Image {
            id: "img_proportional".to_string(),
            path: "".to_string(),
            x: 50.0,
            y: 50.0,
            width: 200.0,
            height: 100.0, // 2:1 aspect ratio
            opacity: 1.0,
            lock_aspect: true,
            rotation: 0.0,
            on_click: None,
        });

        // 2. Freeform Image (lock_aspect: false)
        overlay.elements.push(OverlayElement::Image {
            id: "img_freeform".to_string(),
            path: "".to_string(),
            x: 80.0,
            y: 80.0,
            width: 150.0,
            height: 250.0,
            opacity: 1.0,
            lock_aspect: false,
            rotation: 0.0,
            on_click: None,
        });

        // 3. Interactive control overflowing canvas
        overlay.elements.push(OverlayElement::InteractiveControl {
            id: "ctrl_1".to_string(),
            control: crate::models::InteractiveControlKind::default(),
            color: [255, 255, 255, 255],
            bg_color: [0, 0, 0, 255],
            active_color: [0, 255, 0, 255],
            x: 90.0,
            y: 90.0,
            width: 300.0,
            height: 50.0,
            rotation: 0.0,
        });

        // 4. Text with large font overflowing canvas
        overlay.elements.push(OverlayElement::Text {
            id: "txt_large".to_string(),
            text: "FPS: 144".to_string(),
            font_size: 40.0,
            color: [255, 255, 255, 255],
            x: 40.0,
            y: 40.0,
            width: None,
            height: None,
            lock_aspect: true,
            bold: false,
            font_family: "Segoe UI".to_string(),
            rotation: 0.0,
            on_click: None,
        });

        // 5. Standard Text scaling
        overlay.elements.push(OverlayElement::Text {
            id: "txt_scaled".to_string(),
            text: "Title".to_string(),
            font_size: 20.0,
            color: [255, 255, 255, 255],
            x: 60.0,
            y: 60.0,
            width: None,
            height: None,
            lock_aspect: false,
            bold: true,
            font_family: "Segoe UI".to_string(),
            rotation: 0.0,
            on_click: None,
        });

        // 6. Proportional Shape with locked aspect ratio
        overlay.elements.push(OverlayElement::Shape {
            id: "shape_clamp".to_string(),
            shape_type: crate::models::ShapeKind::Rectangle,
            x: 80.0,
            y: 80.0,
            width: 200.0,
            height: 100.0, // 2:1 aspect ratio
            thickness: 2.0,
            color: [255, 255, 255, 255],
            filled: false,
            fill_color: [0, 0, 0, 0],
            corner_radius: 0.0,
            lock_aspect: true,
            rotation: 0.0,
            on_click: None,
        });

        clamp_elements_to_canvas(&mut overlay);

        // Verify Image 1: proportional shrink (width <= 100, height <= 50, x + width <= 100, y + height <= 100)
        if let OverlayElement::Image { width, height, x, y, .. } = &overlay.elements[1] {
            assert!(*width <= 100.0);
            assert!(*height <= 50.0);
            assert!(*x >= 0.0 && *x + *width <= 100.0);
            assert!(*y >= 0.0 && *y + *height <= 100.0);
        } else {
            panic!("Expected Image element");
        }

        // Verify Image 2: freeform shrink (width <= 100, height <= 100)
        if let OverlayElement::Image { width, height, x, y, .. } = &overlay.elements[2] {
            assert!(*width <= 100.0);
            assert!(*height <= 100.0);
            assert!(*x >= 0.0 && *x + *width <= 100.0);
            assert!(*y >= 0.0 && *y + *height <= 100.0);
        } else {
            panic!("Expected Image element");
        }

        // Verify InteractiveControl: width <= 100, height <= 100, coordinates clamped
        if let OverlayElement::InteractiveControl { width, height, x, y, .. } = &overlay.elements[3] {
            assert!(*width <= 100.0);
            assert!(*height <= 100.0);
            assert!(*x >= 0.0 && *x + *width <= 100.0);
            assert!(*y >= 0.0 && *y + *height <= 100.0);
        } else {
            panic!("Expected InteractiveControl element");
        }

        // Verify Text: the clamp never rewrites font_size (U02-003). An oversized label is moved
        // to the card origin and clipped by the card painter at render time instead.
        if let OverlayElement::Text { font_size, x, y, .. } = &overlay.elements[4] {
            assert_eq!(*font_size, 40.0, "the clamp must not shrink the user's font");
            assert_eq!(*x, 0.0);
            let (_, eh) = text_size_estimate("FPS: 144", *font_size);
            assert!(*y >= 0.0 && *y + eh <= 100.0);
        } else {
            panic!("Expected Text element");
        }

        // Verify Standard Text 5: clamped to canvas
        if let OverlayElement::Text { font_size, x, y, text, .. } = &overlay.elements[5] {
            let (ew, eh) = get_element_size(&OverlayElement::Text {
                id: "".to_string(),
                text: text.clone(),
                font_size: *font_size,
                color: [0; 4],
                x: 0.0,
                y: 0.0,
                width: None,
                height: None,
                lock_aspect: false,
                bold: false,
                font_family: "".to_string(),
                rotation: 0.0,
                on_click: None,
            });
            assert!(*x >= 0.0 && *x + ew <= 100.0);
            assert!(*y >= 0.0 && *y + eh <= 100.0);
        } else {
            panic!("Expected Scaled Text element");
        }

        // Verify Proportional Shape 6: clamped to canvas with preserved aspect ratio (width <= 100, height <= 50)
        if let OverlayElement::Shape { width, height, x, y, .. } = &overlay.elements[6] {
            assert!(*width <= 100.0);
            assert!(*height <= 50.0);
            assert!(*x >= 0.0 && *x + *width <= 100.0);
            assert!(*y >= 0.0 && *y + *height <= 100.0);
        } else {
            panic!("Expected Shape element");
        }
    }

    #[test]
    fn test_monitor_artboard_and_card_aspect_ratio_lock() {
        use crate::models::{OverlayAnchor, OverlayDefinition};

        let mut overlay = OverlayDefinition::default_macro_badge();
        overlay.rect.width = 400.0;
        overlay.rect.height = 200.0;

        // 2. Verify Card Aspect Ratio Lock calculation
        let card_lock_aspect = true;
        let aspect = overlay.rect.width / overlay.rect.height; // 400 / 200 = 2.0
        assert_eq!(aspect, 2.0);

        // Resizing width from 400 to 600 maintains aspect ratio -> height = 300
        let new_w = 600.0;
        let new_h = if card_lock_aspect {
            (new_w / aspect).round()
        } else {
            overlay.rect.height
        };
        assert_eq!(new_h, 300.0);

        // 3. Verify OverlayStudioState default camera pan & zoom, centered default card
        let mut state = GuiState::new();
        state.open_overlay_studio("overlay_custom");
        let studio = state.overlay_studio.as_ref().unwrap();
        assert!(!studio.card_lock_aspect);
        assert_eq!(studio.canvas_pan, egui::Vec2::ZERO);
        assert_eq!(studio.canvas_zoom, None);
        assert_eq!(studio.overlay.anchor, OverlayAnchor::ScreenCenter);
        assert_eq!(studio.overlay.rect.width, 440.0);
        assert_eq!(studio.overlay.rect.height, 200.0);

        // Also verify preset macro badge retains its dedicated top-right geometry
        state.open_overlay_studio("overlay_macro_badge");
        let studio_badge = state.overlay_studio.as_ref().unwrap();
        assert_eq!(studio_badge.overlay.anchor, OverlayAnchor::TopRight);
        assert_eq!(studio_badge.overlay.rect.width, 220.0);
        assert_eq!(studio_badge.overlay.rect.height, 44.0);

        // 7. Verify 4-way auto-panning calculation logic
        let canvas_viewport = Rect::from_min_size(pos2(100.0, 100.0), vec2(800.0, 600.0));
        let edge_zone = 70.0;

        // Pointer near right edge -> pan step X is negative (slides camera right)
        let ptr_right = pos2(canvas_viewport.right() - 20.0, canvas_viewport.center().y);
        let factor_r = ((ptr_right.x - (canvas_viewport.right() - edge_zone)) / edge_zone).clamp(0.0, 1.5);
        let pan_x = -(3.0 + 8.0 * factor_r);
        assert!(pan_x < -3.0);

        // Pointer near left edge -> pan step X is positive (slides camera left)
        let ptr_left = pos2(canvas_viewport.left() + 20.0, canvas_viewport.center().y);
        let factor_l = ((canvas_viewport.left() + edge_zone - ptr_left.x) / edge_zone).clamp(0.0, 1.5);
        let pan_l = 3.0 + 8.0 * factor_l;
        assert!(pan_l > 3.0);

        // Pointer near bottom edge -> pan step Y is negative (slides camera down)
        let ptr_bottom = pos2(canvas_viewport.center().x, canvas_viewport.bottom() - 15.0);
        let factor_b = ((ptr_bottom.y - (canvas_viewport.bottom() - edge_zone)) / edge_zone).clamp(0.0, 1.5);
        let pan_y = -(3.0 + 8.0 * factor_b);
        assert!(pan_y < -3.0);

        // Pointer near top edge -> pan step Y is positive (slides camera up)
        let ptr_top = pos2(canvas_viewport.center().x, canvas_viewport.top() + 15.0);
        let factor_t = ((canvas_viewport.top() + edge_zone - ptr_top.y) / edge_zone).clamp(0.0, 1.5);
        let pan_t = 3.0 + 8.0 * factor_t;
        assert!(pan_t > 3.0);

        // Safeguard: when an element is selected (selected_element_idx is Some), camera auto-pan is disabled
        let selected_element_idx = Some(0usize);
        let pan_step = if selected_element_idx.is_none() {
            vec2(pan_x, pan_y)
        } else {
            Vec2::ZERO
        };
        assert_eq!(pan_step, Vec2::ZERO);

        // 4. Verify max_card_dimensions and clamp_card_to_monitor
        let mut custom_overlay = OverlayDefinition::default_macro_badge();
        custom_overlay.anchor = OverlayAnchor::Custom;
        custom_overlay.rect.x = 1500.0;
        custom_overlay.rect.y = 800.0;
        custom_overlay.rect.width = 600.0;
        custom_overlay.rect.height = 400.0;
        let (max_w, max_h) = max_card_dimensions(&custom_overlay, 1920.0, 1080.0);
        assert_eq!(max_w, 420.0); // 1920 - 1500
        assert_eq!(max_h, 280.0); // 1080 - 800
        clamp_card_to_monitor(&mut custom_overlay, 1920.0, 1080.0);
        assert_eq!(custom_overlay.rect.width, 420.0);
        assert_eq!(custom_overlay.rect.height, 280.0);

        // 6. Verify calibration synchronization matching: when last_synced_pos matches, no overwrite occurs
        let mut studio = state.overlay_studio.unwrap();
        studio.is_calibrating = true;
        studio.overlay.rect.x = 750.0;
        studio.overlay.rect.y = 350.0;
        studio.last_synced_pos = Some((750.0, 350.0));
        let daemon_pos = (750.0, 350.0);
        let pos_differs_from_last_synced = studio.last_synced_pos.is_none_or(|(lx, ly)| (lx - daemon_pos.0).abs() > 0.5 || (ly - daemon_pos.1).abs() > 0.5);
        assert!(!pos_differs_from_last_synced, "Position matching last_synced_pos must not trigger overwrite");
    }

    #[test]
    fn test_multimonitor_canvas_support_and_card_clamping() {
        use crate::models::{MonitorInfo, MonitorOrientation, OverlayDefinition};

        // Create a 2-monitor virtual desktop setup:
        // Monitor 0: Primary Landscape 2560x1440 at (0, 0)
        // Monitor 1: Secondary Portrait 1080x1920 at (-1080, -383)
        let monitors = vec![
            MonitorInfo {
                index: 0,
                name: "Primary Display".to_string(),
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
                is_primary: true,
            },
            MonitorInfo {
                index: 1,
                name: "Portrait Display".to_string(),
                x: -1080,
                y: -383,
                width: 1080,
                height: 1920,
                is_primary: false,
            },
        ];

        // 1. Verify monitor orientation detection
        assert_eq!(monitors[0].orientation(), MonitorOrientation::Landscape);
        assert!(!monitors[0].is_portrait());
        assert_eq!(monitors[1].orientation(), MonitorOrientation::Portrait);
        assert!(monitors[1].is_portrait());

        // 2. Verify resolve_overlay_monitor
        let mut overlay = OverlayDefinition::default_new_overlay();
        assert_eq!(overlay.monitor_index, None);
        let resolved = resolve_overlay_monitor(&overlay, &monitors);
        assert_eq!(resolved.index, 0);
        assert!(resolved.is_primary);

        // Switch to secondary monitor (index 1)
        overlay.monitor_index = Some(1);
        let resolved_sec = resolve_overlay_monitor(&overlay, &monitors);
        assert_eq!(resolved_sec.index, 1);
        assert_eq!(resolved_sec.width, 1080);
        assert_eq!(resolved_sec.height, 1920);

        // Invalid index falls back to primary
        overlay.monitor_index = Some(99);
        let resolved_fallback = resolve_overlay_monitor(&overlay, &monitors);
        assert_eq!(resolved_fallback.index, 0);

        // 3. Verify Virtual Desktop Bounding Box calculation
        let min_x = monitors.iter().map(|m| m.x).min().unwrap_or(0);
        let min_y = monitors.iter().map(|m| m.y).min().unwrap_or(0);
        let max_x = monitors.iter().map(|m| m.x + m.width as i32).max().unwrap_or(1920);
        let max_y = monitors.iter().map(|m| m.y + m.height as i32).max().unwrap_or(1080);
        assert_eq!(min_x, -1080);
        assert_eq!(min_y, -383);
        assert_eq!(max_x, 2560);
        assert_eq!(max_y, 1537); // -383 + 1920 = 1537

        let desktop_w = (max_x - min_x) as f32; // 3640
        let desktop_h = (max_y - min_y) as f32; // 1920
        assert_eq!(desktop_w, 3640.0);
        assert_eq!(desktop_h, 1920.0);

        // 4. Verify strict clamping within active monitor (preventing crossing to other monitors during drag)
        // Set overlay on Monitor 1 (Portrait 1080x1920)
        overlay.monitor_index = Some(1);
        overlay.rect.width = 400.0;
        overlay.rect.height = 300.0;
        let mon1 = &monitors[1];

        // Attempt to drag card beyond right edge of portrait monitor (e.g. 1200.0 px)
        let dragged_rel_x = 1200.0f32;
        let clamped_rel_x = dragged_rel_x.clamp(0.0, (mon1.width as f32 - overlay.rect.width).max(0.0));
        assert_eq!(clamped_rel_x, 680.0); // 1080 - 400 = 680

        // Attempt to drag card below bottom edge of portrait monitor (e.g. 2500.0 px)
        let dragged_rel_y = 2500.0f32;
        let clamped_rel_y = dragged_rel_y.clamp(0.0, (mon1.height as f32 - overlay.rect.height).max(0.0));
        assert_eq!(clamped_rel_y, 1620.0); // 1920 - 300 = 1620

        // Attempt to drag card into negative coordinates (crossing left or top)
        let dragged_neg_x = -150.0f32;
        let clamped_neg_x = dragged_neg_x.clamp(0.0, (mon1.width as f32 - overlay.rect.width).max(0.0));
        assert_eq!(clamped_neg_x, 0.0);

        // Verify card cannot cross monitor boundaries
        assert!(clamped_rel_x + overlay.rect.width <= mon1.width as f32);
        assert!(clamped_rel_y + overlay.rect.height <= mon1.height as f32);
        assert!(clamped_neg_x >= 0.0);

        // 5. Verify Canvas Display Header badge label formatting (catalog value + fill helper)
        let loc = crate::i18n::get(crate::i18n::Language::English);
        let mon0_label = fill(
            loc.overlay_studio_canvas_display,
            &[&monitors[0].width.to_string(), &monitors[0].height.to_string()],
        );
        assert_eq!(mon0_label, "DISPLAY 2560×1440");
        let mon1_label = fill(
            loc.overlay_studio_canvas_display,
            &[&monitors[1].width.to_string(), &monitors[1].height.to_string()],
        );
        assert_eq!(mon1_label, "DISPLAY 1080×1920");

        // 6. Verify Overlay Header Pill text formatting (no glyph, no brackets)
        let pill_text = fill(
            loc.overlay_studio_canvas_overlay,
            &[
                &(overlay.rect.width as i32).to_string(),
                &(overlay.rect.height as i32).to_string(),
            ],
        );
        assert_eq!(pill_text, "OVERLAY 400×300");

        // 7. Verify Image & Card boundary clipping prevents bleeding into left settings panel
        let left_panel_rect = Rect::from_min_max(pos2(0.0, 50.0), pos2(400.0, 800.0));
        let canvas_rect = Rect::from_min_max(pos2(400.0, 50.0), pos2(1200.0, 800.0));
        // Panned card overlapping left panel (left edge at 200.0, crossing x=400.0)
        let panned_card = Rect::from_min_max(pos2(200.0, 100.0), pos2(700.0, 500.0));
        let effective_clip = panned_card.intersect(canvas_rect);
        assert_eq!(effective_clip.min.x, 400.0, "Clip rect must never extend left of canvas boundary");
        assert!(!effective_clip.intersects(left_panel_rect.shrink(0.1)), "Clip rect must not intersect left settings panel");
        assert!(effective_clip.is_positive());
    }

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

    // ---------------- U02-001: element colour targets ----------------

    #[test]
    fn test_element_colour_targets_are_element_scoped() {
        for elem_idx in [0usize, 4] {
            for slot in [ElementColorSlot::Primary, ElementColorSlot::Secondary, ElementColorSlot::Tertiary] {
                let target = element_color_target(elem_idx, slot);
                assert!(
                    !matches!(target, ColorPickerTarget::CardBg | ColorPickerTarget::CardBorder),
                    "an element colour chip must never target the card style"
                );
                assert_eq!(active_element_slot(Some(target), elem_idx), Some(slot));
                assert_eq!(active_element_slot(Some(target), elem_idx + 1), None);
            }
        }
        assert_eq!(active_element_slot(Some(ColorPickerTarget::CardBg), 0), None);
        assert_eq!(active_element_slot(Some(ColorPickerTarget::CardBorder), 0), None);
        assert_eq!(active_element_slot(None, 0), None);
    }

    // ---------------- U02-004: crosshair sentinel ----------------

    #[test]
    fn test_crosshair_coordinates_never_hit_the_centre_sentinel() {
        let range = crosshair_coord_range(100.0);
        assert_eq!(*range.start(), 1.0);
        assert_eq!(*range.end(), 100.0);
        assert_eq!(clamp_crosshair_coord(0.0, 100.0), 1.0);
        assert_eq!(clamp_crosshair_coord(-25.0, 100.0), 1.0);
        assert_eq!(clamp_crosshair_coord(42.0, 100.0), 42.0);
        assert_eq!(clamp_crosshair_coord(500.0, 100.0), 100.0);

        // A legacy overlay saved with the sentinel / a negative value is clamped out of it.
        let mut overlay = OverlayDefinition::default_crosshair();
        overlay.rect.width = 120.0;
        overlay.rect.height = 90.0;
        if let OverlayElement::Crosshair { x, y, .. } = &mut overlay.elements[0] {
            *x = 0.0;
            *y = -12.0;
        }
        clamp_elements_to_canvas(&mut overlay);
        if let OverlayElement::Crosshair { x, y, .. } = &overlay.elements[0] {
            assert!(*x >= 1.0);
            assert!(*y >= 1.0);
        }
    }

    // ---------------- U02-002: preview interactivity ----------------

    #[test]
    fn test_studio_preview_is_not_interactive() {
        use crate::models::InteractiveControlKind;

        let inputs = studio_preview_render_inputs();
        assert!(!inputs.is_interactive, "the studio must not execute element on_click actions");
        assert!(!inputs.pointer_clicked);
        assert!(!inputs.pointer_down);

        // The explicit studio test path still toggles the widget, on the real element.
        let mut elem = OverlayElement::InteractiveControl {
            id: "switch".to_string(),
            control: InteractiveControlKind::ToggleSwitch {
                label: "Toggle".to_string(),
                is_active: false,
                bind_rule_id: None,
                on_enable: None,
                on_disable: None,
            },
            color: [255, 255, 255, 255],
            bg_color: [0, 0, 0, 255],
            active_color: [0, 255, 0, 255],
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 20.0,
            rotation: 0.0,
        };
        studio_test_click(&mut elem);
        if let OverlayElement::InteractiveControl {
            control: InteractiveControlKind::ToggleSwitch { is_active, .. },
            ..
        } = &elem
        {
            assert!(*is_active);
        } else {
            panic!("expected the toggle switch to stay a toggle switch");
        }

        // The countdown starts from its full duration on the first studio click.
        let mut timer = OverlayElement::ClockTimer {
            id: "timer".to_string(),
            mode: crate::models::TimerMode::Countdown,
            format_str: "%H:%M".to_string(),
            countdown_total_ms: 60_000,
            countdown_remaining_ms: 0,
            stopwatch_elapsed_ms: 0,
            is_running: false,
            font_size: 13.0,
            color: [255, 255, 255, 255],
            bg_color: [0, 0, 0, 0],
            bold: false,
            font_family: "Segoe UI".to_string(),
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 24.0,
            rotation: 0.0,
            on_finish: None,
        };
        studio_test_click(&mut timer);
        if let OverlayElement::ClockTimer { is_running, countdown_remaining_ms, .. } = &timer {
            assert!(*is_running);
            assert_eq!(*countdown_remaining_ms, 60_000);
        }
    }

    // ---------------- U02-003: no per-frame clamping ----------------

    #[test]
    fn test_text_font_size_is_not_clamped_by_preview() {
        let mut overlay = OverlayDefinition::default_crosshair();
        overlay.rect.width = 80.0;
        overlay.rect.height = 40.0;
        overlay.elements = vec![OverlayElement::Text {
            id: "label".to_string(),
            text: "A very long label that cannot possibly fit".to_string(),
            font_size: 48.0,
            color: [255, 255, 255, 255],
            x: 0.0,
            y: 0.0,
            width: None,
            height: None,
            lock_aspect: true,
            bold: false,
            font_family: "Segoe UI".to_string(),
            rotation: 0.0,
            on_click: None,
        }];
        clamp_elements_to_canvas(&mut overlay);
        if let OverlayElement::Text { font_size, .. } = &overlay.elements[0] {
            assert_eq!(*font_size, 48.0, "clamping must never rewrite the user's font size");
        } else {
            panic!("expected a Text element");
        }

        // The clamp is idempotent once the geometry was applied.
        let mut once = overlay.clone();
        clamp_elements_to_canvas(&mut once);
        assert_eq!(overlay, once);
    }

    #[test]
    fn test_overlay_geometry_ignores_presentation_only_edits() {
        let overlay = OverlayDefinition::default_crosshair();
        let geometry = overlay_geometry(&overlay);

        let mut recoloured = overlay.clone();
        recoloured.style.bg_color = [1, 2, 3, 4];
        recoloured.opacity = 0.4;
        assert_eq!(
            overlay_geometry(&recoloured),
            geometry,
            "colour/opacity edits must not trigger the geometry clamp"
        );

        let mut moved = overlay.clone();
        *moved.elements[0].x_mut() = 32.0;
        assert_ne!(overlay_geometry(&moved), geometry, "a geometry edit must trigger the clamp");
    }

    #[test]
    fn test_clamp_runs_only_when_geometry_changes() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ctx| {
            let mut overlay = OverlayDefinition::default_crosshair();
            overlay.rect.width = 200.0;
            overlay.rect.height = 200.0;
            overlay.elements.push(OverlayElement::Text {
                id: "label".to_string(),
                text: "Label".to_string(),
                font_size: 24.0,
                color: [255, 255, 255, 255],
                x: 50.0,
                y: 150.0,
                width: None,
                height: None,
                lock_aspect: true,
                bold: false,
                font_family: "Segoe UI".to_string(),
                rotation: 0.0,
                on_click: None,
            });
            clamp_studio_geometry_if_changed(ctx, &mut overlay);
            assert_eq!(overlay.elements[1].x(), 50.0, "the element already fits");

            // Shrinking the card is a real geometry change and must clamp.
            overlay.rect.width = 80.0;
            clamp_studio_geometry_if_changed(ctx, &mut overlay);
            assert!(overlay.elements[1].x() <= 80.0);

            // Typing more text is presentation-only: the font size survives the next frames.
            if let OverlayElement::Text { text, .. } = &mut overlay.elements[1] {
                text.push_str(" with a much longer caption");
            }
            clamp_studio_geometry_if_changed(ctx, &mut overlay);
            clamp_studio_geometry_if_changed(ctx, &mut overlay);
            if let OverlayElement::Text { font_size, .. } = &overlay.elements[1] {
                assert_eq!(*font_size, 24.0);
            }
        });
        out.textures_delta.clear();
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

    // ---------------- U02-007: toggle-rule authoring ----------------

    #[test]
    fn test_rule_click_entry_writes_the_chosen_variant() {
        assert!(matches!(
            rule_click_action("rule-1", false),
            OverlayClickAction::TriggerRule { ref rule_id } if rule_id == "rule-1"
        ));
        assert!(matches!(
            rule_click_action("rule-1", true),
            OverlayClickAction::ToggleRule { ref rule_id } if rule_id == "rule-1"
        ));
    }

    // ---------------- U02-010: shared size limits ----------------

    #[test]
    fn test_size_and_position_limits_stay_in_sync() {
        assert_eq!(IMAGE_SIZE_LIMITS, SizeLimits { min_w: 8.0, min_h: 8.0 });
        assert_eq!(SHAPE_SIZE_LIMITS, SizeLimits { min_w: 4.0, min_h: 4.0 });
        assert_eq!(PANEL_SIZE_LIMITS, SizeLimits { min_w: 20.0, min_h: 15.0 });
        assert_eq!(MEDIA_SIZE_LIMITS, SizeLimits { min_w: 40.0, min_h: 20.0 });
        assert_eq!(KEYVISUALIZER_SIZE_LIMITS, SizeLimits { min_w: 30.0, min_h: 18.0 });

        // The canvas clamp keeps at least the minimum size the inspector rows allow.
        let mut overlay = OverlayDefinition::default_crosshair();
        overlay.rect.width = 200.0;
        overlay.rect.height = 200.0;
        overlay.elements = vec![
            OverlayElement::Image {
                id: "img".to_string(),
                path: String::new(),
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                opacity: 1.0,
                lock_aspect: false,
                rotation: 0.0,
                on_click: None,
            },
            OverlayElement::Shape {
                id: "shape".to_string(),
                shape_type: ShapeKind::Rectangle,
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                thickness: 2.0,
                color: [255, 255, 255, 255],
                filled: false,
                fill_color: [0, 0, 0, 0],
                corner_radius: 0.0,
                lock_aspect: false,
                rotation: 0.0,
                on_click: None,
            },
        ];
        clamp_elements_to_canvas(&mut overlay);
        if let OverlayElement::Image { width, height, .. } = &overlay.elements[0] {
            assert!(*width >= IMAGE_SIZE_LIMITS.min_w);
            assert!(*height >= IMAGE_SIZE_LIMITS.min_h);
        } else {
            panic!("expected an Image element");
        }
        if let OverlayElement::Shape { width, height, .. } = &overlay.elements[1] {
            assert!(*width >= SHAPE_SIZE_LIMITS.min_w);
            assert!(*height >= SHAPE_SIZE_LIMITS.min_h);
        } else {
            panic!("expected a Shape element");
        }
    }

    // ---------------- U02-013: one hit order ----------------

    #[test]
    fn test_canvas_hit_test_order() {
        let card = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 200.0));
        let pill = Rect::from_min_size(pos2(0.0, -20.0), vec2(114.0, 16.0));
        let card_resize_handle = Rect::from_center_size(pos2(200.0, 200.0), vec2(14.0, 14.0));
        let elem_rects = vec![
            (0usize, Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0))),
            (1usize, Rect::from_min_max(pos2(40.0, 40.0), pos2(60.0, 60.0))),
        ];
        let handles = Some(SelectionHandles {
            idx: 1,
            rot_rect: Rect::from_center_size(pos2(10.0, 10.0), vec2(16.0, 16.0)),
            resize_rect: Rect::from_center_size(pos2(80.0, 80.0), vec2(16.0, 16.0)),
            center: pos2(50.0, 50.0),
            half_w: 20.0,
            half_h: 20.0,
        });

        // The selected element's rotate/resize handles win over elements underneath them.
        assert!(matches!(
            canvas_hit_test(pos2(10.0, 10.0), handles, &elem_rects, card_resize_handle, pill, card),
            CanvasHit::Rotate { idx: 1, .. }
        ));
        assert!(matches!(
            canvas_hit_test(pos2(80.0, 80.0), handles, &elem_rects, card_resize_handle, pill, card),
            CanvasHit::Resize { idx: 1 }
        ));
        // Topmost element wins when their rects overlap.
        assert!(matches!(
            canvas_hit_test(pos2(50.0, 50.0), handles, &elem_rects, card_resize_handle, pill, card),
            CanvasHit::Element { idx: 1 }
        ));
        // Without a selection the handles do not exist: the element under them is hit.
        assert!(matches!(
            canvas_hit_test(pos2(55.0, 55.0), None, &elem_rects, card_resize_handle, pill, card),
            CanvasHit::Element { idx: 1 }
        ));
        // Card resize handle, then card body, then empty canvas.
        assert!(matches!(
            canvas_hit_test(pos2(198.0, 198.0), None, &elem_rects, card_resize_handle, pill, card),
            CanvasHit::CardResize
        ));
        assert!(matches!(
            canvas_hit_test(pos2(150.0, 150.0), None, &elem_rects, card_resize_handle, pill, card),
            CanvasHit::CardBody
        ));
        assert!(matches!(
            canvas_hit_test(pos2(500.0, 500.0), None, &elem_rects, card_resize_handle, pill, card),
            CanvasHit::Empty
        ));

        // Starting a drag applies the selection and exactly one undo snapshot; panning pushes none.
        let mut studio = OverlayStudioState::new(OverlayDefinition::default_crosshair(), None, false, String::new());
        let mode = begin_canvas_drag(&mut studio, CanvasHit::Element { idx: 0 }, pos2(5.0, 5.0), 0.0, 0.0);
        assert!(matches!(mode, CanvasDragMode::Move { idx: 0, .. }));
        assert_eq!(studio.selected_element_idx, Some(0));
        assert_eq!(studio.undo_stack.len(), 1);
        let mode = begin_canvas_drag(&mut studio, CanvasHit::Empty, pos2(5.0, 5.0), 0.0, 0.0);
        assert!(matches!(mode, CanvasDragMode::Pan { .. }));
        assert_eq!(studio.selected_element_idx, None);
        assert_eq!(studio.undo_stack.len(), 1);
    }

    // ---------------- U02-014: one badge table ----------------

    #[test]
    fn test_element_badge_covers_every_kind() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let t = Tokens::get(&ctx);
        let color = [255u8; 4];
        let elements = vec![
            OverlayElement::Crosshair {
                id: "c".to_string(),
                style: CrosshairStyle::Cross,
                size: 14.0,
                color,
                thickness: 2.0,
                gap: 4.0,
                center_dot: true,
                x: 10.0,
                y: 10.0,
                rotation: 0.0,
            },
            OverlayElement::Text {
                id: "t".to_string(),
                text: "T".to_string(),
                font_size: 12.0,
                color,
                x: 0.0,
                y: 0.0,
                width: None,
                height: None,
                lock_aspect: true,
                bold: false,
                font_family: "Segoe UI".to_string(),
                rotation: 0.0,
                on_click: None,
            },
            OverlayElement::Image {
                id: "i".to_string(),
                path: String::new(),
                x: 0.0,
                y: 0.0,
                width: 32.0,
                height: 32.0,
                opacity: 1.0,
                lock_aspect: true,
                rotation: 0.0,
                on_click: None,
            },
            OverlayElement::Shape {
                id: "s".to_string(),
                shape_type: ShapeKind::Circle,
                x: 0.0,
                y: 0.0,
                width: 32.0,
                height: 32.0,
                thickness: 2.0,
                color,
                filled: false,
                fill_color: color,
                corner_radius: 0.0,
                lock_aspect: true,
                rotation: 0.0,
                on_click: None,
            },
            OverlayElement::Graph {
                id: "g".to_string(),
                metric: crate::models::GraphMetric::SystemCpu,
                style: crate::models::GraphStyle::Line,
                sample_count: 60,
                color,
                fill_color: color,
                bg_color: color,
                show_current_val: true,
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 40.0,
                rotation: 0.0,
            },
            OverlayElement::ClockTimer {
                id: "clk".to_string(),
                mode: crate::models::TimerMode::Clock,
                format_str: "%H:%M".to_string(),
                countdown_total_ms: 60_000,
                countdown_remaining_ms: 60_000,
                stopwatch_elapsed_ms: 0,
                is_running: false,
                font_size: 13.0,
                color,
                bg_color: color,
                bold: false,
                font_family: "Segoe UI".to_string(),
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 24.0,
                rotation: 0.0,
                on_finish: None,
            },
            OverlayElement::InteractiveControl {
                id: "ctl".to_string(),
                control: crate::models::InteractiveControlKind::Button {
                    label: "Go".to_string(),
                    icon: None,
                    action: None,
                },
                color,
                bg_color: color,
                active_color: color,
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 20.0,
                rotation: 0.0,
            },
            OverlayElement::SystemTelemetry {
                id: "tel".to_string(),
                kind: crate::models::TelemetryKind::Battery,
                style: crate::models::TelemetryStyle::Badge,
                color,
                bg_color: color,
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 28.0,
                rotation: 0.0,
                on_click: None,
            },
            OverlayElement::MediaWidget {
                id: "media".to_string(),
                show_controls: true,
                compact: false,
                color,
                bg_color: color,
                x: 0.0,
                y: 0.0,
                width: 220.0,
                height: 54.0,
                rotation: 0.0,
            },
            OverlayElement::KeyVisualizer {
                id: "keys".to_string(),
                layout: crate::models::KeyVisualizerLayout::WasdMouse,
                max_keys: 4,
                decay_seconds: 2.5,
                show_modifiers_only: false,
                color,
                bg_color: color,
                active_color: color,
                x: 0.0,
                y: 0.0,
                width: 170.0,
                height: 105.0,
                rotation: 0.0,
            },
        ];
        let loc = crate::i18n::get(crate::i18n::Language::English);
        let expected = [
            loc.overlay_studio_badge_crosshair,
            loc.overlay_studio_badge_text,
            loc.overlay_studio_badge_image,
            loc.overlay_studio_badge_circle,
            loc.overlay_studio_badge_graph,
            loc.overlay_studio_badge_clock,
            loc.overlay_studio_badge_control,
            loc.overlay_studio_badge_status,
            loc.overlay_studio_badge_media,
            loc.overlay_studio_badge_keys,
        ];
        for (elem, label) in elements.iter().zip(expected) {
            let (badge, _) = element_badge(elem, &t, loc);
            assert_eq!(badge, label);
        }
        // Shape variants share the label table with the card header.
        let mut arrow = elements[3].clone();
        if let OverlayElement::Shape { shape_type, .. } = &mut arrow {
            *shape_type = ShapeKind::Arrow;
        }
        assert_eq!(element_badge(&arrow, &t, loc).0, loc.overlay_studio_badge_arrow);

        // The badge table follows the active language, so a Russian studio does not show the
        // English badge.
        let ru = crate::i18n::get(crate::i18n::Language::Russian);
        assert_eq!(
            element_badge(&elements[0], &t, ru).0,
            ru.overlay_studio_badge_crosshair
        );
    }

    // ---------------- U02-012: studio strings come from the catalog ----------------

    #[test]
    fn test_studio_catalog_values_drive_the_rendered_labels() {
        let en = crate::i18n::get(crate::i18n::Language::English);
        let ru = crate::i18n::get(crate::i18n::Language::Russian);

        // Field prefixes and units carry their own separator/space in the catalog.
        assert_eq!(en.overlay_studio_field_screen_x, "Screen X: ");
        assert_eq!(ru.overlay_studio_field_screen_x, "Экран X: ");
        assert_eq!(en.overlay_studio_field_w, "W: ");
        assert_eq!(ru.overlay_studio_field_w, "Ш: ");
        assert_eq!(en.overlay_studio_unit_px, " px");
        assert_eq!(en.overlay_studio_unit_pt, " pt");

        // Colour chips and palette swatches are localised.
        assert_eq!(en.overlay_studio_chip_bg, "BG:");
        assert_eq!(ru.overlay_studio_chip_bg, "ФОН:");
        assert_eq!(en.overlay_studio_chip_text, "TEXT:");
        assert_eq!(ru.overlay_studio_chip_text, "ТЕКСТ:");
        assert_ne!(en.overlay_studio_palette_cyan, ru.overlay_studio_palette_cyan);
        assert_ne!(en.overlay_studio_crosshair_cross, ru.overlay_studio_crosshair_cross);

        // Badges, control defaults and the preset fallback are catalog values, not literals.
        assert_ne!(en.overlay_studio_badge_crosshair, ru.overlay_studio_badge_crosshair);
        assert_eq!(en.overlay_studio_control_default_action, "Action");
        assert_eq!(ru.overlay_studio_control_default_action, "Действие");
        assert_eq!(en.overlay_default_preset_name, "Custom Overlay");
        assert_eq!(ru.overlay_default_preset_name, "Свой оверлей");
        assert_eq!(en.overlay_studio_on_click_toggle, "Toggle");
        assert_eq!(ru.overlay_studio_on_click_toggle, "Переключить");
        assert_eq!(en.overlay_studio_file_filter_image, "Image");
        assert_eq!(ru.overlay_studio_file_filter_image, "Изображение");

        // Preset save/delete failure toasts.
        assert_eq!(
            en.presets_save_invalid_name,
            "Preset name needs at least one letter or digit"
        );
        assert_eq!(en.presets_save_failed, "Could not save the preset; see the log");
        assert_eq!(
            en.presets_delete_failed,
            "Could not delete the preset; see the log"
        );
        assert_ne!(en.presets_save_failed, ru.presets_save_failed);
        assert_ne!(en.presets_delete_failed, ru.presets_delete_failed);
        assert_ne!(en.presets_save_invalid_name, ru.presets_save_invalid_name);

        // Multi-placeholder canvas labels are filled positionally.
        assert_eq!(
            fill(en.overlay_studio_canvas_display, &["1920", "1080"]),
            "DISPLAY 1920×1080"
        );
        assert_eq!(
            fill(ru.overlay_studio_canvas_display, &["1920", "1080"]),
            "МОНИТОР 1920×1080"
        );
        assert_eq!(fill(en.overlay_studio_canvas_overlay, &["400", "300"]), "OVERLAY 400×300");
        assert_eq!(
            fill(en.overlay_studio_canvas_coords, &["12", "34", "56", "78", "90"]),
            "X: 12 Y: 34 | 56×78 px | 90°"
        );
    }

    /// The production part of one studio source file: everything before its `#[cfg(test)]`
    /// module, with comment lines dropped.
    fn production_part(source: &'static str) -> String {
        let production = source
            .split_once("#[cfg(test)]")
            .map(|(production, _)| production)
            .unwrap_or(source);
        production
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Guards the conversion: the English studio literals replaced by `Strings` fields must not
    /// come back into production code. The test module itself contains these strings, so the
    /// scan stops at `#[cfg(test)]` and ignores comment lines.
    #[test]
    fn test_no_hardcoded_studio_english_literals_remain() {
        let production = [
            production_part(include_str!("overlay_modal.rs")),
            production_part(include_str!("../state/library_write.rs")),
        ]
        .join("\n");

        let denied = [
            // field prefixes / units
            r#""W: ""#,
            r#""H: ""#,
            r#""X: ""#,
            r#""Y: ""#,
            r#""Screen X: ""#,
            r#""Screen Y: ""#,
            r#""Width: ""#,
            r#""Height: ""#,
            r#""Border: ""#,
            r#""Radius: ""#,
            r#""Size: ""#,
            r#""Thick: ""#,
            r#""Gap: ""#,
            r#""Min: ""#,
            r#""Max: ""#,
            r#"" px""#,
            r#"" pt""#,
            // colour chip labels
            r#""BG:""#,
            r#""BORDER:""#,
            r#""COLOR:""#,
            r#""TEXT COLOR:""#,
            r#""TEXT:""#,
            r#""FILL:""#,
            r#""LINE:""#,
            r#""ACTIVE:""#,
            r#""GLOW:""#,
            // palette swatch names
            r#""Cyan""#,
            r#""Green""#,
            r#""Red""#,
            r#""Amber""#,
            r#""White""#,
            r#""Dark""#,
            r#""Clear""#,
            // crosshair styles and clock panel
            r#""Cross""#,
            r#""T-Shape""#,
            r#""Circle""#,
            r#""Dot""#,
            r#""Font Size""#,
            r#""Bold""#,
            // control kind defaults
            r#""Action""#,
            r#""Volume""#,
            r#""Button""#,
            r#""Toggle""#,
            r#""Slider""#,
            // element badges
            r#""CROSSHAIR""#,
            r#""TEXT""#,
            r#""IMAGE""#,
            r#""SQUARE""#,
            r#""CIRCLE""#,
            r#""ARROW""#,
            r#""LINE""#,
            r#""GRAPH""#,
            r#""CLOCK""#,
            r#""CONTROL""#,
            r#""STATUS""#,
            r#""MEDIA""#,
            r#""KEYS""#,
            // presets, filter, on-click and canvas labels
            r#""Custom Overlay""#,
            r#""Preset name needs at least one letter or digit""#,
            r#""Could not save the preset; see the log""#,
            r#""Could not delete the preset; see the log""#,
            r#""Image""#,
            r#""DISPLAY {}×{}""#,
            r#""OVERLAY {}×{}""#,
            r#""RULE_CLICK_TOGGLE_LABEL""#,
        ];
        for literal in denied {
            assert!(
                !production.contains(literal),
                "hardcoded overlay-studio string {literal} must come from Strings"
            );
        }
    }

    // ---------------- U02-015: rotation normalisation ----------------

    #[test]
    fn test_rotation_normalisation_matches_canvas() {
        assert_eq!(normalize_rotation_deg(-90.0), 270.0);
        assert_eq!(normalize_rotation_deg(270.0), 270.0);
        assert_eq!(normalize_rotation_deg(360.0), 0.0);
        assert_eq!(normalize_rotation_deg(-720.0), 0.0);
        assert!((normalize_rotation_deg(45.5) - 45.5).abs() < f32::EPSILON);
    }
}
