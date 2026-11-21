//! Precision studio widgets: mono HUD labels, square buttons with hover inversion,
//! segmented hairline cell groups, section eyebrows, and custom window chrome.

use std::sync::Arc;
use egui::{
    Color32, CornerRadius, Galley, Rect, Response, Sense, Stroke, Ui, pos2, vec2,
};
use crate::gui::theme::{self, Tokens};

pub use super::duration::{
    DurationField, TimeUnit, duration_millis_u32, duration_millis_u64, duration_secs_f32,
    duration_secs_f64, duration_secs_u32, duration_secs_u64,
};

/// `text` laid out uppercase in `family`, `spacing` points between letters.
pub fn caps(
    ui: &Ui,
    text: &str,
    size: f32,
    family: egui::FontFamily,
    spacing: f32,
    color: Color32,
) -> Arc<Galley> {
    let format = egui::TextFormat {
        font_id: egui::FontId::new(size, family),
        color,
        extra_letter_spacing: spacing,
        ..Default::default()
    };
    ui.painter().layout_job(egui::text::LayoutJob::single_section(text.to_uppercase(), format))
}

/// A `.hud-label`: 11 px Geist Mono, uppercase, widely spaced.
pub fn hud_galley(ui: &Ui, text: &str, color: Color32) -> Arc<Galley> {
    caps(ui, text, theme::HUD_SIZE, theme::mono(), theme::HUD_SPACING, color)
}

/// Paints `galley` at the left of `rect`, centered vertically.
pub fn paint_left(ui: &Ui, rect: Rect, galley: Arc<Galley>, color: Color32) {
    let pos = pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
    ui.painter().galley_with_override_text_color(pos, galley, color);
}

/// Paints `galley` centered in `rect`.
pub fn paint_centred(ui: &Ui, rect: Rect, galley: Arc<Galley>, color: Color32) {
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, color);
}

/// A precision mono HUD label.
pub fn hud(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let galley = hud_galley(ui, text, color);
    let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::hover());
    paint_left(ui, rect, galley, color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
    resp
}

/// "01 / AUTOMATIONS": a section eyebrow, the index and separator faint.
pub fn eyebrow(ui: &mut Ui, index: &str, label: &str) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        hud(ui, index, t.faint);
        hud(ui, " / ", t.faint);
        hud(ui, label, t.muted);
    });
}

/// A small square status badge: 10 px mono on sunken surface with hairline border.
pub fn badge(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = caps(ui, text, 10.0, theme::mono(), 1.0, color);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 12.0, 20.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::ZERO, t.sunken, t.hairline(), egui::StrokeKind::Inside);
    paint_centred(ui, rect, galley, color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
    resp
}

/// A vertical hairline divider.
pub fn rule(ui: &mut Ui, height: f32) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), t.hairline());
}

/// A horizontal hairline divider across available width.
pub fn hrule(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, t.hairline());
}

/// Which button variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Solid block in inverted colors (white bg in dark theme); dims to 80% on hover.
    Primary,
    /// Outlined; inverts on hover.
    Secondary,
    /// Solid red; dims on hover.
    Danger,
}

/// Fill, border and ink resolved for one button paint pass.
struct KindPaint {
    fill: Color32,
    stroke: Stroke,
    ink: Color32,
}

/// Resolves a [`Kind`] once so [`button`] and [`compact_button`] always paint a variant
/// the same way. Compact buttons live in dense bars, so their resting `Secondary` fill
/// uses the recessed surface; every other variant is identical for both callers.
fn kind_paint(kind: Kind, t: &Tokens, hovered: bool, compact: bool) -> KindPaint {
    let dim = if hovered { 0.85 } else { 1.0 };
    match kind {
        Kind::Primary => KindPaint {
            fill: t.invert_bg.gamma_multiply(dim),
            stroke: Stroke::NONE,
            ink: t.invert_fg,
        },
        Kind::Danger => KindPaint {
            fill: t.bad.gamma_multiply(dim),
            stroke: Stroke::NONE,
            ink: Color32::WHITE,
        },
        Kind::Secondary if hovered => KindPaint {
            fill: t.invert_bg,
            stroke: Stroke::NONE,
            ink: t.invert_fg,
        },
        Kind::Secondary => KindPaint {
            fill: if compact { t.sunken } else { t.bg },
            stroke: Stroke::new(1.0, t.line_strong),
            ink: t.ink,
        },
    }
}

/// The paint pass shared by both button helpers: fades disabled content, fills the
/// frame, centres the galley and registers the label with egui.
fn paint_kind_button(
    ui: &Ui,
    resp: &Response,
    galley: Arc<Galley>,
    text: &str,
    enabled: bool,
    paint: KindPaint,
) {
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.4) };
    ui.painter().rect(
        resp.rect,
        CornerRadius::ZERO,
        fade(paint.fill),
        Stroke::new(paint.stroke.width, fade(paint.stroke.color)),
        egui::StrokeKind::Inside,
    );
    paint_centred(ui, resp.rect, galley, fade(paint.ink));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, text));
}

/// Applies the shared hover cursor to an enabled button response.
fn finish_kind_button(resp: Response, enabled: bool) -> Response {
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Helper to calculate exact button width including padding and spacing
pub fn button_width(ui: &Ui, text: &str, min_width: f32) -> f32 {
    let t = Tokens::get(ui.ctx());
    let galley = caps(ui, text, theme::HUD_SIZE, theme::mono_bold(), theme::BUTTON_SPACING, t.ink);
    (galley.size().x + 28.0).max(min_width)
}

/// A square button with a mono uppercase label, at least `width` wide.
pub fn button(ui: &mut Ui, text: &str, kind: Kind, width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = caps(ui, text, theme::HUD_SIZE, theme::mono_bold(), theme::BUTTON_SPACING, t.ink);
    let size = vec2((galley.size().x + 28.0).max(width), theme::CONTROL_H);
    let (_, resp) = ui.allocate_exact_size(size, Sense::click());
    let enabled = ui.is_enabled();
    let hovered = resp.hovered() && enabled;

    let paint = kind_paint(kind, &t, hovered, false);
    paint_kind_button(ui, &resp, galley, text, enabled, paint);
    finish_kind_button(resp, enabled)
}

/// Full-width outlined action button with plus glyph and tracking, 1-in-1 matching ADD STEP / ADD CONDITION.
pub fn full_width_action_button(
    ui: &mut Ui,
    label: &str,
    tip: &str,
    t: &Tokens,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    let is_hov = resp.hovered();
    let p = ui.painter();

    let border = if is_hov {
        Stroke::new(1.0, t.accent_ink)
    } else {
        Stroke::new(1.0, t.muted.gamma_multiply(0.35))
    };
    p.rect_stroke(rect, CornerRadius::same(2), border, egui::StrokeKind::Inside);

    let text_color = if is_hov { t.accent_ink } else { t.faint };
    let plus_galley = p.layout_no_wrap(
        egui_phosphor::regular::PLUS.to_string(),
        egui::FontId::new(12.0, theme::phosphor()),
        text_color,
    );
    let label_galley = caps(ui, label, 10.0, theme::mono(), 0.8, text_color);

    let spacing = 8.0;
    let plus_w = plus_galley.size().x;
    let plus_h = plus_galley.size().y;
    let total_w = plus_w + spacing + label_galley.size().x;
    let start_x = rect.center().x - total_w * 0.5;

    let plus_pos = pos2(start_x, rect.center().y - plus_h * 0.5);
    p.galley(plus_pos, plus_galley, text_color);

    let label_pos = pos2(start_x + plus_w + spacing, rect.center().y - label_galley.size().y * 0.5);
    p.galley_with_override_text_color(label_pos, label_galley, text_color);

    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let resp = if !tip.trim().is_empty() { resp.on_hover_text(tip) } else { resp };

    resp.clicked()
}

/// Precision toggle switch with smooth animated track and thumb, and hover feedback.
pub fn toggle_switch(ui: &mut Ui, on: &mut bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let width = 38.0;
    let height = 20.0;
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let enabled = ui.is_enabled();
    let hovered = resp.hovered() && enabled;

    // Smooth ease-out animation for on/off transition
    let how_on = ui.ctx().animate_bool_responsive(resp.id, *on);

    let bg_color = {
        let off_bg = if hovered { t.line_strong } else { t.sunken };
        let on_bg = if hovered { t.accent.gamma_multiply(0.85) } else { t.accent };
        let r = egui::lerp((off_bg.r() as f32)..=(on_bg.r() as f32), how_on) as u8;
        let g = egui::lerp((off_bg.g() as f32)..=(on_bg.g() as f32), how_on) as u8;
        let b = egui::lerp((off_bg.b() as f32)..=(on_bg.b() as f32), how_on) as u8;
        let a = egui::lerp((off_bg.a() as f32)..=(on_bg.a() as f32), how_on) as u8;
        Color32::from_rgba_unmultiplied(r, g, b, a)
    };

    let border_stroke = if how_on >= 0.98 {
        Stroke::NONE
    } else {
        let border_color = if hovered { t.line_strong } else { t.line };
        Stroke::new(1.0, border_color.gamma_multiply(1.0 - how_on))
    };

    let corner_radius = CornerRadius::same(height as u8 / 2);
    ui.painter().rect(rect, corner_radius, bg_color, border_stroke, egui::StrokeKind::Inside);

    let thumb_radius = (height - 6.0) / 2.0;
    let min_x = rect.left() + 3.0 + thumb_radius;
    let max_x = rect.right() - 3.0 - thumb_radius;
    let x_pos = egui::lerp(min_x..=max_x, how_on);
    let center = pos2(x_pos, rect.center().y);

    let thumb_color = if how_on > 0.5 {
        Color32::WHITE
    } else if hovered {
        t.ink
    } else {
        t.muted
    };

    ui.painter().circle_filled(center, thumb_radius, thumb_color);

    if enabled {
        resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, *on, enabled, ""));
    resp
}

/// A compact button with specified height (e.g. for status/footer bars).
pub fn compact_button(ui: &mut Ui, text: &str, kind: Kind, height: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = caps(ui, text, 10.0, theme::mono_bold(), theme::BUTTON_SPACING, t.ink);
    let size = vec2(galley.size().x + 14.0, height);
    let (_, resp) = ui.allocate_exact_size(size, Sense::click());
    let enabled = ui.is_enabled();
    let hovered = resp.hovered() && enabled;

    let paint = kind_paint(kind, &t, hovered, true);
    paint_kind_button(ui, &resp, galley, text, enabled, paint);
    finish_kind_button(resp, enabled)
}

/// One option in a segmented [`cells`] group.
pub struct Cell<'a> {
    pub label: &'a str,
    pub tip: &'a str,
    pub count: Option<(usize, Option<Color32>)>,
}

impl<'a> Cell<'a> {
    pub fn new(label: &'a str, tip: &'a str) -> Self {
        Self {
            label,
            tip,
            count: None,
        }
    }
}

/// Mutually exclusive options in a row of hairline cells. The chosen one is inverted.
pub fn cells<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, Cell<'_>)]) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    let mut rects = Vec::with_capacity(options.len());

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (opt, cell) in options {
            let on = *value == *opt;
            let label = hud_galley(ui, cell.label, t.ink);
            let count = cell.count.map(|(n, highlight)| {
                let c = if on {
                    t.invert_fg.gamma_multiply(0.65)
                } else {
                    highlight.filter(|_| n > 0).unwrap_or(t.faint)
                };
                (hud_galley(ui, &n.to_string(), c), c)
            });
            let w = label.size().x + count.as_ref().map_or(0.0, |(g, _)| g.size().x + 8.0) + 24.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(w, theme::CONTROL_H), Sense::click());
            let hovered = resp.hovered();

            let (fill, ink) = match (on, hovered) {
                (true, _) => (t.invert_bg, t.invert_fg),
                (false, true) => (t.sunken, t.ink_strong),
                (false, false) => (Color32::TRANSPARENT, t.muted),
            };

            ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
            let text_rect = rect.shrink2(vec2(12.0, 0.0));
            let label_w = label.size().x;
            paint_left(ui, text_rect, label, ink);
            if let Some((g, c)) = count {
                paint_left(ui, text_rect.with_min_x(text_rect.left() + label_w + 8.0), g, c);
            }

            let resp = if !cell.tip.trim().is_empty() {
                resp.on_hover_text(cell.tip)
            } else {
                resp
            };
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                *value = *opt;
                changed = true;
            }
            rects.push(rect);
        }
    });

    if let (Some(first), Some(last)) = (rects.first(), rects.last()) {
        let p = ui.painter();
        p.rect_stroke(first.union(*last), CornerRadius::ZERO, t.hairline(), egui::StrokeKind::Inside);
        let (top, bottom) = (first.top(), first.bottom());
        for r in rects.iter().skip(1) {
            p.vline(r.left(), top..=bottom, t.hairline());
        }
    }

    changed
}

const RESIZE_EDGE: f32 = 5.0;

/// Custom title bar dragging and double-click to maximize/restore.
pub fn title_bar_drag(ui: &mut Ui) {
    let r = ui.max_rect();
    let rect = Rect::from_min_max(r.min + vec2(RESIZE_EDGE, RESIZE_EDGE), r.max - vec2(RESIZE_EDGE, 0.0));
    let resp = ui.interact(rect, ui.scope_id().with("title-bar-drag"), Sense::click_and_drag());
    if resp.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if resp.double_clicked() {
        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowButton {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// Minimize, maximize (or restore) and close buttons for custom title bars.
pub fn window_buttons(ui: &mut Ui, height: f32, loc: &crate::i18n::Strings) {
    let ctx = ui.ctx().clone();
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    if window_button(ui, WindowButton::Close, height, loc.win_btn_close).clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    let (size, label) = if maximized {
        (WindowButton::Restore, loc.win_btn_restore)
    } else {
        (WindowButton::Maximize, loc.win_btn_maximize)
    };
    if window_button(ui, size, height, label).clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
    if window_button(ui, WindowButton::Minimize, height, loc.win_btn_minimize).clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }
}

fn window_button(ui: &mut Ui, button: WindowButton, height: f32, tip: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(46.0, height), Sense::hover());
    let right = if button == WindowButton::Close { RESIZE_EDGE } else { 0.0 };
    let hit = Rect::from_min_max(rect.min + vec2(0.0, RESIZE_EDGE), rect.max - vec2(right, 0.0));
    let resp = ui.interact(hit, ui.scope_id().with(("window-button", button as u8)), Sense::click());

    let (fill, ink) = match (button, resp.hovered()) {
        (WindowButton::Close, true) => (Color32::from_rgb(0xc4, 0x2b, 0x1c), Color32::WHITE),
        (_, true) => (t.line, t.ink_strong),
        (_, false) => (Color32::TRANSPARENT, t.muted),
    };

    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::ZERO, fill);
    let c = rect.center();
    let stroke = Stroke::new(1.0, ink);

    let s = 5.0;
    match button {
        WindowButton::Close => paint_line_glyph(p, Glyph::Close, c, s, stroke),
        WindowButton::Minimize => paint_line_glyph(p, Glyph::Minimize, c, s, stroke),
        WindowButton::Maximize => {
            p.rect_stroke(Rect::from_center_size(c, vec2(2.0 * s, 2.0 * s)), CornerRadius::ZERO, stroke, egui::StrokeKind::Inside);
        }
        WindowButton::Restore => {
            let front = Rect::from_min_size(c + vec2(-s, -s + 2.0), vec2(2.0 * s - 2.0, 2.0 * s - 2.0));
            p.rect_stroke(front, CornerRadius::ZERO, stroke, egui::StrokeKind::Inside);
            p.hline(c.x - s + 2.0..=c.x + s, c.y - s, stroke);
            p.vline(c.x + s, c.y - s..=c.y + s - 2.0, stroke);
        }
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    if !tip.trim().is_empty() {
        resp.on_hover_text(tip)
    } else {
        resp
    }
}

/// Custom title bars: hairline around the window and resizing from edges and corners.
pub fn window_edges(ctx: &egui::Context) {
    use egui::CursorIcon as C;
    use egui::viewport::ResizeDirection as D;
    let (maximized, fullscreen) = ctx.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().fullscreen.unwrap_or(false)));
    if maximized || fullscreen {
        return;
    }
    let t = Tokens::get(ctx);
    let r = ctx.content_rect();
    ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("window-border"))).rect_stroke(
        r,
        CornerRadius::ZERO,
        t.hairline(),
        egui::StrokeKind::Inside,
    );
    let Some(p) = ctx.input(|i| i.pointer.hover_pos()).filter(|p| r.contains(*p)) else { return };
    let near = |d: f32| d < RESIZE_EDGE;
    let corner = |d: f32| d < 3.0 * RESIZE_EDGE;
    let (left, right, top, bottom) = (p.x - r.left(), r.right() - p.x, p.y - r.top(), r.bottom() - p.y);
    let hit = if (near(top) && corner(left)) || (near(left) && corner(top)) {
        Some((D::NorthWest, C::ResizeNorthWest))
    } else if (near(top) && corner(right)) || (near(right) && corner(top)) {
        Some((D::NorthEast, C::ResizeNorthEast))
    } else if (near(bottom) && corner(left)) || (near(left) && corner(bottom)) {
        Some((D::SouthWest, C::ResizeSouthWest))
    } else if (near(bottom) && corner(right)) || (near(right) && corner(bottom)) {
        Some((D::SouthEast, C::ResizeSouthEast))
    } else if near(top) {
        Some((D::North, C::ResizeNorth))
    } else if near(bottom) {
        Some((D::South, C::ResizeSouth))
    } else if near(left) {
        Some((D::West, C::ResizeWest))
    } else if near(right) {
        Some((D::East, C::ResizeEast))
    } else {
        None
    };
    if let Some((direction, cursor)) = hit {
        ctx.set_cursor_icon(cursor);
        // Do not steal a press that egui is already routing to a widget under the
        // pointer: the inspector's scrollbar sits inside the right edge band, and
        // starting a window resize there would break scrolling.
        if ctx.input(|i| i.pointer.primary_pressed()) && !ctx.egui_is_using_pointer() {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Close,
    Minimize,
    Clone,
    Expand,
    Reset,
    Play,
    Stop,
    Edit,
    Share,
    Shortcut,
    Trash,
    Question,
}

impl Glyph {
    /// The Phosphor codepoint for icon-font variants; `None` for glyphs drawn from
    /// primitives. Kept in one place so the studio buttons and title bars cannot drift.
    fn phosphor(self) -> Option<&'static str> {
        match self {
            Self::Reset => Some(egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE),
            Self::Play => Some(egui_phosphor::regular::PLAY),
            Self::Stop => Some(egui_phosphor::regular::STOP),
            Self::Edit => Some(egui_phosphor::regular::PENCIL_SIMPLE),
            Self::Share => Some(egui_phosphor::regular::SHARE_NETWORK),
            Self::Shortcut => Some(egui_phosphor::regular::ARROW_SQUARE_OUT),
            Self::Trash => Some(egui_phosphor::regular::TRASH),
            Self::Question => Some(egui_phosphor::regular::QUESTION_MARK),
            Self::Close | Self::Minimize | Self::Clone | Self::Expand => None,
        }
    }
}

/// Draws the line glyphs (`Close`, `Minimize`); `half` is the distance from the centre
/// to the stroke ends, so callers can keep their own button size.
fn paint_line_glyph(p: &egui::Painter, glyph: Glyph, centre: egui::Pos2, half: f32, stroke: Stroke) {
    match glyph {
        Glyph::Close => {
            p.line_segment([centre + vec2(-half, -half), centre + vec2(half, half)], stroke);
            p.line_segment([centre + vec2(-half, half), centre + vec2(half, -half)], stroke);
        }
        Glyph::Minimize => {
            p.line_segment([centre + vec2(-half, 0.0), centre + vec2(half, 0.0)], stroke);
        }
        _ => {}
    }
}

/// Draws the overlapping-squares (`Clone`) and expand-arrow (`Expand`) glyphs.
fn paint_box_glyph(p: &egui::Painter, glyph: Glyph, m: egui::Pos2, size: f32, color: Color32) {
    match glyph {
        Glyph::Clone => {
            let back = Rect::from_center_size(m + vec2(-2.0, -2.0), vec2(7.0, 7.0));
            let front = Rect::from_center_size(m + vec2(1.8, 1.8), vec2(7.0, 7.0));
            p.line_segment([back.left_top(), back.right_top()], Stroke::new(1.2, color));
            p.line_segment([back.left_top(), back.left_bottom()], Stroke::new(1.2, color));
            p.line_segment([back.right_top(), back.right_top() + vec2(0.0, 2.0)], Stroke::new(1.2, color));
            p.line_segment([back.left_bottom(), back.left_bottom() + vec2(2.0, 0.0)], Stroke::new(1.2, color));
            p.rect_stroke(front, CornerRadius::ZERO, Stroke::new(1.2, color), egui::StrokeKind::Inside);
        }
        Glyph::Expand => {
            let s = (size * 0.14).clamp(3.2, 4.2);
            p.line_segment([m + vec2(s - 3.2, -s), m + vec2(s, -s)], Stroke::new(1.3, color));
            p.line_segment([m + vec2(s, -s), m + vec2(s, -s + 3.2)], Stroke::new(1.3, color));
            p.line_segment([m + vec2(-s, s - 3.2), m + vec2(-s, s)], Stroke::new(1.3, color));
            p.line_segment([m + vec2(-s, s), m + vec2(-s + 3.2, s)], Stroke::new(1.3, color));
            p.line_segment([m + vec2(0.5, -0.5), m + vec2(s - 0.5, -s + 0.5)], Stroke::new(1.3, color));
            p.line_segment([m + vec2(-0.5, 0.5), m + vec2(-s + 0.5, s - 0.5)], Stroke::new(1.3, color));
        }
        _ => {}
    }
}

/// Draws every icon-font glyph in one pass.
fn paint_font_glyph(p: &egui::Painter, glyph: Glyph, m: egui::Pos2, size: f32, color: Color32) {
    let Some(text) = glyph.phosphor() else { return };
    let font_size = (size * 0.48).clamp(12.0, 15.0);
    p.text(
        m,
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::new(font_size, theme::phosphor()),
        color,
    );
}

/// Small outlined square button with a drawn glyph (X, Clone, Expand, More, Reset); inverts on hover.
pub fn glyph_button(ui: &mut Ui, glyph: Glyph, tip: &str) -> Response {
    glyph_button_active(ui, glyph, false, tip)
}

/// Small outlined square button with a drawn glyph; supports active state.
pub fn glyph_button_active(ui: &mut Ui, glyph: Glyph, active: bool, tip: &str) -> Response {
    let size = if ui.spacing().interact_size.y < theme::CONTROL_H {
        ui.spacing().interact_size.y
    } else {
        theme::CONTROL_H
    };
    glyph_button_custom(ui, glyph, size, active, tip)
}

/// Small outlined square button with custom size and active state.
pub fn glyph_button_custom(ui: &mut Ui, glyph: Glyph, size: f32, active: bool, tip: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hovered = resp.hovered();
    let p = ui.painter();
    let (fill, stroke, c) = if active || hovered {
        (t.invert_bg, Stroke::NONE, t.invert_fg)
    } else {
        (t.bg, Stroke::new(1.0, t.line_strong), t.ink)
    };
    p.rect(rect, CornerRadius::ZERO, fill, stroke, egui::StrokeKind::Inside);
    let m = rect.center();
    match glyph {
        Glyph::Close => paint_line_glyph(p, glyph, m, (size * 0.15).clamp(3.5, 4.5), Stroke::new(1.5, c)),
        Glyph::Minimize => paint_line_glyph(p, glyph, m, (size * 0.18).clamp(4.0, 5.0), Stroke::new(1.5, c)),
        Glyph::Clone | Glyph::Expand => paint_box_glyph(p, glyph, m, size, c),
        _ => paint_font_glyph(p, glyph, m, size, c),
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    let resp = if !tip.trim().is_empty() {
        resp.on_hover_text(tip)
    } else {
        resp
    };
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A precision drag handle with 6 studio dots; senses click and drag.
pub fn drag_handle(ui: &mut Ui, tip: &str) -> Response {
    drag_handle_custom(ui, theme::CONTROL_H, tip)
}

/// A precision drag handle with customizable height and 6 studio dots; senses click and drag.
pub fn drag_handle_custom(ui: &mut Ui, height: f32, tip: &str) -> Response {
    drag_handle_custom_interactive(ui, height, tip, true)
}

/// A precision drag handle with customizable height, interactive toggle, and 6 studio dots.
pub fn drag_handle_custom_interactive(ui: &mut Ui, height: f32, tip: &str, interactive: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let sense = if interactive { Sense::click_and_drag() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(16.0, height), sense);
    let hovered = resp.hovered() && interactive;
    let dragged = resp.dragged() && interactive;
    let p = ui.painter();
    if hovered || dragged {
        p.rect_filled(rect, CornerRadius::ZERO, t.sunken);
    }
    let c = if dragged {
        t.accent_ink
    } else if hovered {
        t.ink
    } else if interactive {
        t.muted.gamma_multiply(0.6)
    } else {
        t.faint.gamma_multiply(0.4)
    };
    let m = rect.center();
    let dx = 2.8;
    let dy = 3.8;
    for x in [-dx, dx] {
        for y in [-dy, 0.0, dy] {
            p.circle_filled(m + vec2(x, y), 1.2, c);
        }
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    let cursor = if !interactive {
        egui::CursorIcon::NotAllowed
    } else if dragged {
        egui::CursorIcon::Grabbing
    } else {
        egui::CursorIcon::Grab
    };
    let resp = if !tip.trim().is_empty() {
        resp.on_hover_text(tip)
    } else {
        resp
    };
    resp.on_hover_cursor(cursor)
}

/// App wordmark in display font with accent color suffix, vertically centered in container of given height.
pub fn wordmark(ui: &mut Ui, prefix: &str, suffix: &str, size: f32, height: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let mut job = egui::text::LayoutJob::default();
    let format = |color| egui::TextFormat {
        font_id: egui::FontId::new(size, theme::display()),
        color,
        extra_letter_spacing: -0.03 * size,
        ..Default::default()
    };
    job.append(prefix, 0.0, format(t.ink_strong));
    job.append(suffix, 0.0, format(t.accent_ink));
    let galley = ui.painter().layout_job(job);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x, height), Sense::hover());
    let pos = pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, Color32::WHITE);
    resp
}

/// Groups vertical checkboxes with tight, compact desktop line spacing (~20px height, 4px vertical gap).
pub fn checkbox_group<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let old_h = ui.spacing().interact_size.y;
    let old_spacing_y = ui.spacing().item_spacing.y;
    ui.spacing_mut().interact_size.y = 20.0;
    ui.spacing_mut().item_spacing.y = 4.0;
    let ret = add_contents(ui);
    ui.spacing_mut().interact_size.y = old_h;
    ui.spacing_mut().item_spacing.y = old_spacing_y;
    ret
}

/// A compact checkbox that does not inflate to 34px CONTROL_H.
pub fn checkbox(ui: &mut Ui, checked: &mut bool, text: impl Into<egui::WidgetText>) -> Response {
    let old_h = ui.spacing().interact_size.y;
    ui.spacing_mut().interact_size.y = 20.0;
    let resp = ui.checkbox(checked, text);
    ui.spacing_mut().interact_size.y = old_h;
    resp
}

/// A horizontal row for a checkbox with trailing inline controls (e.g. DragValue, buttons, notes),
/// guaranteeing that the checkbox and all trailing controls are vertically centered on the exact same line.
pub fn checkbox_row<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let old_interact_h = ui.spacing().interact_size.y;
    let old_spacing_x = ui.spacing().item_spacing.x;
    let old_padding = ui.spacing().button_padding;

    ui.spacing_mut().button_padding = egui::vec2(6.0, 2.0);
    ui.spacing_mut().interact_size.y = 20.0;
    ui.spacing_mut().item_spacing.x = 8.0;

    let row_h = 24.0;
    let ret = ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), row_h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| add_contents(ui),
    ).inner;

    ui.spacing_mut().button_padding = old_padding;
    ui.spacing_mut().interact_size.y = old_interact_h;
    ui.spacing_mut().item_spacing.x = old_spacing_x;
    ret
}

/// A sleek, rounded catalog search input with an embedded magnifying glass icon at the start,
/// corner radius matching catalog cards (CornerRadius::same(6)), an integrated clear button,
/// and subtle focus highlighting.
pub fn catalog_search_box(
    ui: &mut Ui,
    query: &mut String,
    hint_text: &str,
) -> Response {
    let t = Tokens::get(ui.ctx());
    let search_id = ui.scope_id().with("catalog_search_box_input");
    let is_focused = ui.ctx().memory(|m| m.has_focus(search_id));

    let search_w = ui.available_width().max(60.0);
    let (box_rect, box_resp) = ui.allocate_exact_size(vec2(search_w, 32.0), Sense::click());
    if box_resp.clicked() {
        ui.ctx().memory_mut(|m| m.request_focus(search_id));
    }

    let is_hovered = box_resp.hovered();
    let border_stroke = if is_focused {
        Stroke::new(1.0, t.accent_ink)
    } else if is_hovered {
        Stroke::new(1.0, t.line_strong)
    } else {
        Stroke::new(1.0, t.line)
    };

    ui.painter().rect_filled(box_rect, CornerRadius::same(6), t.bg);
    ui.painter().rect_stroke(box_rect, CornerRadius::same(6), border_stroke, egui::StrokeKind::Inside);

    // Magnifying glass icon on left, perfectly centered vertically
    let icon_pos = pos2(box_rect.left() + 11.0, box_rect.center().y);
    let icon_color = if is_focused { t.accent_ink } else { t.muted };
    ui.painter().text(
        icon_pos,
        egui::Align2::LEFT_CENTER,
        egui_phosphor::regular::MAGNIFYING_GLASS,
        egui::FontId::new(15.0, theme::phosphor()),
        icon_color,
    );

    let has_query = !query.is_empty();
    let clear_w = if has_query { 28.0 } else { 10.0 };

    // Text edit area between icon and clear button, spanning full height to center text vertically
    let text_rect = Rect::from_min_max(
        pos2(box_rect.left() + 32.0, box_rect.top() + 1.0),
        pos2(box_rect.right() - clear_w, box_rect.bottom() + 1.0),
    );

    let edit_resp = ui.put(
        text_rect,
        egui::TextEdit::singleline(query)
            .id(search_id)
            .hint_text(
                egui::RichText::new(hint_text)
                    .family(theme::medium())
                    .size(12.5)
                    .color(t.faint),
            )
            .font(egui::FontId::new(12.5, theme::medium()))
            .text_color(t.ink)
            .frame(egui::Frame::NONE)
            .align(egui::Align2::LEFT_CENTER),
    );

    // Clear button on the right
    if has_query {
        let clear_rect = Rect::from_center_size(
            pos2(box_rect.right() - 15.0, box_rect.center().y),
            vec2(18.0, 18.0),
        );
        let clear_resp = ui.interact(clear_rect, ui.scope_id().with("catalog_search_clear"), Sense::click());
        let clear_hovered = clear_resp.hovered();
        if clear_hovered {
            ui.painter().rect_filled(clear_rect, CornerRadius::same(3), t.sunken);
        }
        ui.painter().text(
            clear_rect.center(),
            egui::Align2::CENTER_CENTER,
            egui_phosphor::regular::X,
            egui::FontId::new(12.0, theme::phosphor()),
            if clear_hovered { t.ink } else { t.muted },
        );
        if clear_resp.clicked() {
            query.clear();
            ui.ctx().memory_mut(|m| m.request_focus(search_id));
        }
    }

    edit_resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_paint_resolves_every_variant_for_both_button_sizes() {
        let t = Tokens::DARK;

        let primary = kind_paint(Kind::Primary, &t, false, false);
        assert_eq!(primary.fill, t.invert_bg);
        assert_eq!(primary.ink, t.invert_fg);
        assert_eq!(primary.stroke, Stroke::NONE);
        // The compact helper used to paint Primary with the accent colour instead.
        let primary_compact = kind_paint(Kind::Primary, &t, false, true);
        assert_eq!(primary_compact.fill, t.invert_bg);
        assert_eq!(primary_compact.ink, t.invert_fg);
        assert_ne!(kind_paint(Kind::Primary, &t, true, false).fill, t.invert_bg);

        let secondary = kind_paint(Kind::Secondary, &t, false, false);
        assert_eq!(secondary.fill, t.bg);
        assert_eq!(secondary.ink, t.ink);
        assert_eq!(secondary.stroke.width, 1.0);
        assert_eq!(kind_paint(Kind::Secondary, &t, false, true).fill, t.sunken);
        let secondary_hovered = kind_paint(Kind::Secondary, &t, true, false);
        assert_eq!(secondary_hovered.fill, t.invert_bg);
        assert_eq!(secondary_hovered.ink, t.invert_fg);

        let danger = kind_paint(Kind::Danger, &t, false, true);
        assert_eq!(danger.fill, t.bad);
        assert_eq!(danger.ink, Color32::WHITE);
    }

    #[test]
    fn glyph_phosphor_mapping_covers_icon_font_variants() {
        for glyph in [
            Glyph::Reset,
            Glyph::Play,
            Glyph::Stop,
            Glyph::Edit,
            Glyph::Share,
            Glyph::Shortcut,
            Glyph::Trash,
            Glyph::Question,
        ] {
            assert!(glyph.phosphor().is_some(), "{glyph:?} should use the icon font");
        }
        for glyph in [Glyph::Close, Glyph::Minimize, Glyph::Clone, Glyph::Expand] {
            assert_eq!(glyph.phosphor(), None, "{glyph:?} should be drawn from primitives");
        }
    }

    fn run_edge_frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        scrollbar_like_widget: bool,
    ) -> Vec<egui::ViewportCommand> {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            if scrollbar_like_widget {
                // Stand-in for the inspector scrollbar inside the right resize band.
                let rect = Rect::from_min_max(pos2(790.0, 200.0), pos2(800.0, 400.0));
                let _ = ui.allocate_rect(rect, Sense::click_and_drag());
            }
            window_edges(ui.ctx());
        });
        output.textures_delta.clear();
        output
            .viewport_output
            .values()
            .flat_map(|viewport| viewport.commands.iter().cloned())
            .collect()
    }

    #[test]
    fn window_edges_defers_begin_resize_to_a_widget_using_the_pointer() {
        let pointer = pos2(797.0, 300.0);
        let press = vec![egui::Event::PointerButton {
            pos: pointer,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }];

        // A widget was hovered last frame and now receives the press: no resize.
        let ctx = egui::Context::default();
        let _ = run_edge_frame(&ctx, vec![], true);
        let _ = run_edge_frame(&ctx, vec![egui::Event::PointerMoved(pointer)], true);
        let commands = run_edge_frame(&ctx, press.clone(), true);
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::BeginResize(_))),
            "a widget under the pointer must keep the press instead of resizing the window"
        );

        // The bare border still starts a resize.
        let ctx = egui::Context::default();
        let _ = run_edge_frame(&ctx, vec![egui::Event::PointerMoved(pointer)], false);
        let commands = run_edge_frame(&ctx, press, false);
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::BeginResize(_))),
            "the bare border must still start a resize"
        );
    }
}
