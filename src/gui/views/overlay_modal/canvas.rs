//! Overlay studio canvas: monitor artboard projection, element painting, hit-testing, the drag
//! state machine and the non-interactive preview helpers.

use egui::{
    pos2, vec2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2,
};

use crate::gui::components::text::fill;
use crate::gui::state::OverlayStudioState;
use crate::gui::theme::{self, Tokens};
use crate::models::{compute_overlay_screen_pos, OverlayAnchor, OverlayElement, ShapeKind};

use super::geometry::{
    clamp_crosshair_coord, element_center_on_canvas, get_rotated_aabb, rotate_point, rotate_vec,
};
use super::{
    clamp_card_to_monitor, clamp_elements_to_canvas, get_element_size, max_card_dimensions,
    resolve_overlay_monitor,
};

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

/// Badge label and colour for one element kind, shared by the inspector card header and the
/// floating drag preview (U02-014).
pub(super) fn element_badge(elem: &OverlayElement, t: &Tokens, loc: &crate::i18n::Strings) -> (&'static str, Color32) {
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

pub(super) fn render_overlay_canvas(
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
        .corner_radius(t.inset_corner())
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
    use crate::models::{CrosshairStyle, OverlayDefinition};

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
}
