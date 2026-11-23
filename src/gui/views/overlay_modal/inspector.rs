//! Overlay studio inspector: the card panel, the drag-reorder element list and one property
//! panel per element kind, plus the shared property rows they use.

use egui::{vec2, FontId, Rect, RichText, Sense, Stroke, Ui};

use crate::gui::components::color_picker::inline_color_editor;
use crate::gui::components::dnd;

use crate::gui::components::widgets::{self, Glyph, Kind};
use crate::gui::state::{ColorPickerTarget, OverlayStudioState};
use crate::gui::theme::{self, Tokens};
use crate::models::{
    compute_overlay_screen_pos, compute_overlay_screen_pos_with_anchor, CrosshairStyle,
    OverlayAnchor, OverlayClickAction, OverlayDefinition, OverlayElement, ShapeKind,
};

use super::canvas::element_badge;
use super::colors::{
    active_element_slot, element_color_target, render_color_chip, ElementColorSlot,
};
use super::geometry::{crosshair_coord_range, text_size_estimate};
use super::{
    clamp_card_to_monitor, clamp_elements_to_canvas, get_element_size, max_card_dimensions,
    resolve_overlay_monitor,
};
use super::history::push_overlay_undo;

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

#[derive(Clone, Copy, Debug, Default)]
struct ElementDndSession {
    source_idx: usize,
    target_idx: usize,
}

/// Overlay element list drop line: 20 px, 0.16 s, rail inset 20/8 and a plain 10 px bead. The
/// corner follows the skin, like the other drop indicators.
fn elem_drop_line_style(t: &Tokens) -> dnd::DropLineStyle {
    dnd::DropLineStyle {
        height: 20.0,
        anim_secs: 0.16,
        corner: t.chip_corner(),
        fill_alpha: 0.08,
        line_width: 2.0,
        line_left: 20.0,
        line_right: 8.0,
        bead: Some(dnd::DropBead {
            left: 10.0,
            radius: 3.5,
            halo_radius: None,
        }),
    }
}

/// Element drag preview chrome: a flat card with a 1 px accent stroke.
fn elem_preview_style(t: &Tokens) -> dnd::FloatingPreviewStyle {
    dnd::FloatingPreviewStyle {
        corner: t.inset_corner(),
        stroke_width: 1.0,
        shadow: false,
    }
}

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
    dnd::drop_line(ui, anim_id, is_targeted, &elem_drop_line_style(t), t);
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
    let (rect, resp) = ui.allocate_exact_size(vec2(theme::control_height(ui), theme::control_height(ui)), Sense::click());
    if locked {
        ui.painter().rect_filled(rect, t.chip_corner(), t.accent_ink.gamma_multiply(0.2));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, t.chip_corner(), t.sunken);
    }
    ui.painter().rect_stroke(rect, t.chip_corner(), t.hairline(), egui::StrokeKind::Inside);
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
                let btn_w = theme::control_height(ui);
                let spacing = ui.spacing().item_spacing.x;
                let field_w = ((total_w - btn_w - spacing * 2.0 - 4.0) / 2.0).max(40.0).floor();
                let w_resp = ui.add_sized(
                    vec2(field_w, theme::control_height(ui)),
                    egui::DragValue::new(width)
                        .range(limits.min_w..=canvas_w.max(limits.min_w))
                        .prefix(loc.overlay_studio_field_w)
                        .suffix(loc.overlay_studio_unit_px)
                        .speed(1.0),
                );
                let h_resp = ui.add_sized(
                    vec2(field_w, theme::control_height(ui)),
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

pub(super) fn render_overlay_inspector(
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
    let btn_w = theme::control_height(ui);
    let spacing = ui.spacing().item_spacing.x;
    let field_w = ((total_w - btn_w - spacing * 2.0 - 4.0) / 2.0).max(40.0).floor();

    let prev_w = overlay.rect.width;
    let prev_h = overlay.rect.height;
    let aspect = if prev_h > 0.0 { prev_w / prev_h } else { 1.0 };

    let mut dim_changed = false;
    ui.horizontal(|ui| {
        let w_resp = ui.add_sized(
            vec2(field_w, theme::control_height(ui)),
            egui::DragValue::new(&mut overlay.rect.width)
                .range(20.0..=max_w)
                .prefix(loc.overlay_studio_field_width)
                .suffix(loc.overlay_studio_unit_px)
                .speed(1.0),
        );
        let h_resp = ui.add_sized(
            vec2(field_w, theme::control_height(ui)),
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
            ColorPickerTarget::CardBg => inline_color_editor(ui, &mut overlay.style.bg_color, true, t, loc),
            ColorPickerTarget::CardBorder => inline_color_editor(ui, &mut overlay.style.border_color, true, t, loc),
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
            .corner_radius(t.inset_corner())
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
                    &elem_preview_style(t),
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
        inline_color_editor(ui, color, false, t, loc);
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
        inline_color_editor(ui, color, true, t, loc);
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
                .min_size(vec2(0.0, theme::control_height(ui)))
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
        Some(ElementColorSlot::Primary) => inline_color_editor(ui, color, true, t, loc),
        Some(ElementColorSlot::Secondary) if is_fillable => inline_color_editor(ui, fill_color, true, t, loc),
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
        Some(ElementColorSlot::Primary) => inline_color_editor(ui, color, true, t, loc),
        Some(ElementColorSlot::Secondary) => inline_color_editor(ui, fill_color, true, t, loc),
        Some(ElementColorSlot::Tertiary) => inline_color_editor(ui, bg_color, true, t, loc),
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
        Some(ElementColorSlot::Primary) => inline_color_editor(ui, color, true, t, loc),
        Some(ElementColorSlot::Secondary) => inline_color_editor(ui, bg_color, true, t, loc),
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
        Some(ElementColorSlot::Primary) => inline_color_editor(ui, color, true, t, loc),
        Some(ElementColorSlot::Secondary) => inline_color_editor(ui, bg_color, true, t, loc),
        Some(ElementColorSlot::Tertiary) => inline_color_editor(ui, active_color, true, t, loc),
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
        Some(ElementColorSlot::Primary) => inline_color_editor(ui, color, true, t, loc),
        Some(ElementColorSlot::Secondary) => inline_color_editor(ui, bg_color, true, t, loc),
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
        Some(ElementColorSlot::Primary) => inline_color_editor(ui, color, true, t, loc),
        Some(ElementColorSlot::Secondary) => inline_color_editor(ui, bg_color, true, t, loc),
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
        Some(ElementColorSlot::Primary) => inline_color_editor(ui, color, true, t, loc),
        Some(ElementColorSlot::Secondary) => inline_color_editor(ui, bg_color, true, t, loc),
        Some(ElementColorSlot::Tertiary) => inline_color_editor(ui, active_color, true, t, loc),
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

#[cfg(test)]
mod tests {
    use super::*;

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
