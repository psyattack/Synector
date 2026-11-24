//! Precision studio widgets: mono HUD labels, square buttons with hover inversion,
//! segmented hairline cell groups, section eyebrows, and custom window chrome.

use std::sync::Arc;
use egui::{
    Color32, CornerRadius, Galley, Rect, Response, Sense, Stroke, Ui, pos2, vec2,
};
use crate::config::UiSkin;
use crate::gui::theme::{self, Tokens};

pub use super::duration::{
    DurationField, TimeUnit, duration_millis_u32, duration_millis_u64, duration_secs_f32,
    duration_secs_f64, duration_secs_u32, duration_secs_u64,
};

/// `text` laid out uppercase in `family`, `spacing` points between letters.
///
/// The voice belongs to the design language, not to the skin: both generations keep the studio
/// typography (mono caps labels, Archivo display), so a skin only changes geometry, surfaces and
/// motion.
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
    ui.painter()
        .layout_job(egui::text::LayoutJob::single_section(text.to_uppercase(), format))
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

/// "01 / SECTION": a section eyebrow, the index and separator faint.
pub fn eyebrow(ui: &mut Ui, index: &str, label: &str) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        hud(ui, index, t.faint);
        hud(ui, " / ", t.faint);
        hud(ui, label, t.muted);
    });
}

/// A section heading in the page-title voice of the in-app guide: display family, strong ink,
/// a touch smaller than the guide's 17 px page title so it sits calmly inside the editor drawer.
pub fn section_heading(ui: &mut Ui, text: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .family(theme::display())
                .size(15.5)
                .color(t.ink_strong),
        )
        .wrap()
        .wrap_mode(egui::TextWrapMode::Wrap),
    )
}

/// A small status badge: a chip in the label voice of the skin.
pub fn badge(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = caps(ui, text, 10.0, theme::mono(), 1.0, color);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 12.0, 20.0), Sense::hover());
    let fill = if t.metrics().skin == UiSkin::Classic {
        t.sunken
    } else {
        t.raised()
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(t.metrics().control_radius),
        fill,
        t.hairline(),
        egui::StrokeKind::Inside,
    );
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
    /// Paint the one-pixel inner top highlight that reads as a glass edge.
    gloss: bool,
}

/// Resolves a [`Kind`] once so [`button`] and [`compact_button`] always paint a variant
/// the same way. `hover` is the eased 0..1 pointer state, so the transition is smooth without
/// any view having to animate it. Compact buttons live in dense bars, so their resting
/// `Secondary` fill uses the recessed surface.
fn kind_paint(kind: Kind, t: &Tokens, hover: f32, compact: bool) -> KindPaint {
    if t.metrics().skin == UiSkin::Modern {
        // The modern generation: a monochrome call to action, lift-based secondary, no accent
        // slabs. The accent is reserved for focus and selection.
        return match kind {
            Kind::Primary => KindPaint {
                // The page colour is mixed in instead of lifting the fill through gamma: a gamma
                // lift cannot dim the black primary of the light palette, so its hover would be
                // invisible there.
                fill: t.ink_strong.lerp_to_gamma(t.bg, 0.16 * hover),
                stroke: Stroke::NONE,
                ink: t.bg,
                gloss: false,
            },
            Kind::Danger => KindPaint {
                fill: t.bad.gamma_multiply(1.0 + 0.12 * hover),
                stroke: Stroke::NONE,
                ink: Color32::WHITE,
                gloss: false,
            },
            Kind::Secondary => KindPaint {
                // A translucent tint instead of an opaque grey slab: the page wash shows
                // through, and the hover is felt in the border and the label.
                fill: t.glass_button(hover),
                stroke: Stroke::new(
                    t.metrics().hairline_w,
                    t.line.lerp_to_gamma(t.line_strong, 0.55 + 0.45 * hover),
                ),
                ink: t.ink.lerp_to_gamma(t.ink_strong, hover),
                gloss: true,
            },
        };
    }

    let dim = 1.0 - 0.15 * hover;
    match kind {
        Kind::Primary => KindPaint {
            fill: t.invert_bg.gamma_multiply(dim),
            stroke: Stroke::NONE,
            ink: t.invert_fg,
            gloss: false,
        },
        Kind::Danger => KindPaint {
            fill: t.bad.gamma_multiply(dim),
            stroke: Stroke::NONE,
            ink: Color32::WHITE,
            gloss: false,
        },
        Kind::Secondary => {
            let rest = if compact { t.sunken } else { t.bg };
            KindPaint {
                fill: rest.lerp_to_gamma(t.invert_bg, hover),
                stroke: Stroke::new(1.0, t.line_strong.gamma_multiply(1.0 - hover)),
                ink: t.ink.lerp_to_gamma(t.invert_fg, hover),
                gloss: false,
            }
        }
    }
}

/// The paint pass shared by both button helpers: fades disabled content, fills the
/// frame, centres the galley and registers the label with egui.
fn paint_kind_button(
    ui: &Ui,
    t: &Tokens,
    resp: &Response,
    galley: Arc<Galley>,
    text: &str,
    enabled: bool,
    paint: KindPaint,
) {
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.4) };
    ui.painter().rect(
        resp.rect,
        CornerRadius::same(t.metrics().control_radius),
        fade(paint.fill),
        Stroke::new(paint.stroke.width, fade(paint.stroke.color)),
        egui::StrokeKind::Inside,
    );
    if paint.gloss {
        // A lit inner edge, inset by the corner radius so it never crosses the rounding.
        let inset = t.metrics().control_radius.min(6) as f32;
        let top = resp.rect.top() + 1.0;
        ui.painter().hline(
            resp.rect.x_range().shrink(inset),
            top,
            Stroke::new(1.0, fade(t.surface_highlight())),
        );
    }
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
    let height = Tokens::get(ui.ctx()).metrics().control_h;
    button_with_height(ui, text, kind, width, height)
}

/// Same button at an explicit height. Bars whose content must not follow the skin's control row
/// (the rule editor's footer keeps one row height in both generations) pass their own height.
pub fn button_with_height(ui: &mut Ui, text: &str, kind: Kind, width: f32, height: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = caps(ui, text, theme::HUD_SIZE, theme::mono_bold(), theme::BUTTON_SPACING, t.ink);
    let size = vec2((galley.size().x + 28.0).max(width), height);
    let (_, resp) = ui.allocate_exact_size(size, Sense::click());
    let enabled = ui.is_enabled();
    let hovered = resp.hovered() && enabled;

    // One eased hover value per button: the transition lives in the theme layer, not in views.
    let hover_seconds = if theme::motion::scroll_in_progress(ui.ctx()) { 0.0 } else { t.metrics().motion.fast };
    let hover = theme::motion::animate_bool(ui.ctx(), resp.id.with("kind_hover"), hovered, hover_seconds);
    let paint = kind_paint(kind, &t, hover, false);
    paint_kind_button(ui, &t, &resp, galley, text, enabled, paint);
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
        Stroke::new(t.metrics().hairline_w, t.accent_ink)
    } else {
        Stroke::new(t.metrics().hairline_w, t.muted.gamma_multiply(0.35))
    };
    let radius = CornerRadius::same(t.metrics().control_radius);
    p.rect_stroke(rect, radius, border, egui::StrokeKind::Inside);

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

/// Toggle switch with an animated track and thumb, sized and voiced by the active skin.
pub fn toggle_switch(ui: &mut Ui, on: &mut bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let metrics = t.metrics();
    let (width, height) = (metrics.toggle_w, metrics.toggle_h);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let enabled = ui.is_enabled();
    let hovered = resp.hovered() && enabled;

    // The skin owns the duration, the OS owns whether motion is allowed at all.
    let how_on = theme::motion::animate_bool(
        ui.ctx(),
        resp.id,
        *on,
        metrics.motion.fast,
    );

    let bg_color = {
        let off_bg = if hovered {
            t.line_strong
        } else {
            if metrics.skin == UiSkin::Modern {
                t.raised()
            } else {
                t.sunken
            }
        };
        let on_bg = if hovered { t.accent.gamma_multiply(0.85) } else { t.accent };
        off_bg.lerp_to_gamma(on_bg, how_on)
    };

    let border_stroke = if how_on >= 0.98 {
        Stroke::NONE
    } else {
        let border_color = if hovered { t.line_strong } else { t.line };
        Stroke::new(metrics.hairline_w, border_color.gamma_multiply(1.0 - how_on))
    };

    let corner_radius = CornerRadius::same((height * 0.5) as u8);
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

    let hover_seconds = if theme::motion::scroll_in_progress(ui.ctx()) { 0.0 } else { t.metrics().motion.fast };
    let hover = theme::motion::animate_bool(ui.ctx(), resp.id.with("kind_hover"), hovered, hover_seconds);
    let paint = kind_paint(kind, &t, hover, true);
    paint_kind_button(ui, &t, &resp, galley, text, enabled, paint);
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

/// Mutually exclusive options in a row of cells. The classic generation inverts the chosen
/// cell; the modern one slides a lifted pill to it and drops the internal dividers.
pub fn cells<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, Cell<'_>)]) -> bool {
    let t = Tokens::get(ui.ctx());
    let metrics = t.metrics();
    let classic = metrics.skin == UiSkin::Classic;
    let radius = CornerRadius::same(metrics.control_radius);
    let mut changed = false;
    /// One laid-out segment: its rect, its label, an optional count and the ink it must be
    /// painted with. Painting happens after the sliding indicator, so the pill stays behind
    /// the text.
    struct PlacedCell {
        rect: Rect,
        label: Arc<Galley>,
        count: Option<(Arc<Galley>, Color32)>,
        ink: Color32,
        on: bool,
    }

    let mut placed: Vec<PlacedCell> = Vec::with_capacity(options.len());

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (opt, cell) in options {
            let on = *value == *opt;
            let label = hud_galley(ui, cell.label, t.ink);
            let count = cell.count.map(|(n, highlight)| {
                let c = if on {
                    if classic {
                        t.invert_fg.gamma_multiply(0.65)
                    } else {
                        t.muted
                    }
                } else {
                    highlight.filter(|_| n > 0).unwrap_or(t.faint)
                };
                (hud_galley(ui, &n.to_string(), c), c)
            });
            let w = label.size().x + count.as_ref().map_or(0.0, |(g, _)| g.size().x + 8.0) + 24.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(w, metrics.control_h), Sense::click());
            let hovered = resp.hovered();

            let ink = if on || hovered { t.ink_strong } else { t.muted };
            if classic && (on || hovered) {
                let fill = if on { t.invert_bg } else { t.sunken };
                ui.painter().rect_filled(rect, radius, fill);
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
            let ink = if on && classic { t.invert_fg } else { ink };
            placed.push(PlacedCell { rect, label, count, ink, on });
        }
    });

    for cell in &placed {
        // The modern chosen segment is simply lifted; there is no slide between options.
        if cell.on && !classic {
            ui.painter().rect_filled(cell.rect.shrink(1.0), radius, t.raised_hover());
        }
        let text_rect = cell.rect.shrink2(vec2(12.0, 0.0));
        let label_w = cell.label.size().x;
        paint_left(ui, text_rect, cell.label.clone(), cell.ink);
        if let Some((g, c)) = &cell.count {
            paint_left(ui, text_rect.with_min_x(text_rect.left() + label_w + 8.0), g.clone(), *c);
        }
    }

    if let (Some(first), Some(last)) = (placed.first(), placed.last()) {
        let p = ui.painter();
        p.rect_stroke(first.rect.union(last.rect), radius, t.hairline(), egui::StrokeKind::Inside);
        if classic {
            let (top, bottom) = (first.rect.top(), first.rect.bottom());
            for cell in placed.iter().skip(1) {
                p.vline(cell.rect.left(), top..=bottom, t.hairline());
            }
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
    /// The header analytics dialog trigger.
    Analytics,
    /// The header settings dialog trigger.
    Settings,
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
            Self::Analytics => Some(egui_phosphor::regular::CHART_DONUT),
            Self::Settings => Some(egui_phosphor::regular::GEAR_SIX),
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

/// Draws every icon-font glyph in one pass at an explicit icon size.
fn paint_font_glyph(p: &egui::Painter, glyph: Glyph, m: egui::Pos2, font_size: f32, color: Color32) {
    let Some(text) = glyph.phosphor() else { return };
    p.text(
        m,
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::new(font_size, theme::phosphor()),
        color,
    );
}

/// Small outlined icon button with a drawn glyph (X, Clone, Expand, More, Reset).
pub fn glyph_button(ui: &mut Ui, glyph: Glyph, tip: &str) -> Response {
    let control_h = Tokens::get(ui.ctx()).metrics().control_h;
    let size = if ui.spacing().interact_size.y < control_h {
        ui.spacing().interact_size.y
    } else {
        control_h
    };
    glyph_button_custom(ui, glyph, size, false, tip)
}

/// The same icon button without a resting surface: only the hover paints. Used by dialog
/// headers and title bars, where a column of grey squares would compete with the content.
pub fn glyph_button_quiet(ui: &mut Ui, glyph: Glyph, tip: &str) -> Response {
    let size = Tokens::get(ui.ctx()).metrics().control_h;
    glyph_button_impl(ui, glyph, size, None, false, GlyphChrome::Quiet, tip)
}

/// An outlined icon button with an explicit icon size and no fill at all: only the border marks
/// the button in both skins. The header navigation uses it, so the glyph pair sits on the same
/// surface as the title bar instead of on two grey slabs.
pub fn glyph_button_outlined(
    ui: &mut Ui,
    glyph: Glyph,
    size: f32,
    icon_size: f32,
    tip: &str,
) -> Response {
    glyph_button_impl(ui, glyph, size, Some(icon_size), false, GlyphChrome::Outlined, tip)
}

#[derive(Clone, Copy)]
enum GlyphChrome {
    /// The standard button surface.
    Framed,
    /// No chrome at rest; only the hover paints.
    Quiet,
    /// Transparent at every state; a hairline border is the only surface.
    Outlined,
}

/// Icon button with custom size and active state.
pub fn glyph_button_custom(ui: &mut Ui, glyph: Glyph, size: f32, active: bool, tip: &str) -> Response {
    glyph_button_impl(ui, glyph, size, None, active, GlyphChrome::Framed, tip)
}

fn glyph_button_impl(
    ui: &mut Ui,
    glyph: Glyph,
    size: f32,
    icon_size: Option<f32>,
    active: bool,
    chrome: GlyphChrome,
    tip: &str,
) -> Response {
    let t = Tokens::get(ui.ctx());
    let metrics = t.metrics();
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hovered = resp.hovered();
    let p = ui.painter();
    let (fill, stroke, c) = match chrome {
        // No fill in either skin: the border alone marks the button, and hover only strengthens
        // the outline and the ink.
        GlyphChrome::Outlined => (
            Color32::TRANSPARENT,
            if hovered {
                Stroke::new(metrics.hairline_w, t.line_strong)
            } else {
                t.hairline()
            },
            if hovered { t.ink_strong } else { t.ink },
        ),
        GlyphChrome::Quiet if metrics.skin == UiSkin::Modern => {
            // No resting chrome at all; the hover supplies both the tint and the outline.
            if hovered {
                (
                    t.glass_button(1.0),
                    Stroke::new(metrics.hairline_w, t.line_strong),
                    t.ink_strong,
                )
            } else {
                (Color32::TRANSPARENT, Stroke::NONE, t.ink)
            }
        }
        GlyphChrome::Framed if metrics.skin == UiSkin::Modern => {
            if active {
                (
                    t.glass_button(1.0),
                    Stroke::new(metrics.hairline_w, t.line_strong),
                    t.ink_strong,
                )
            } else {
                (
                    t.glass_button(if hovered { 1.0 } else { 0.0 }),
                    Stroke::new(
                        metrics.hairline_w,
                        t.line.lerp_to_gamma(t.line_strong, if hovered { 1.0 } else { 0.0 }),
                    ),
                    if hovered { t.ink_strong } else { t.ink },
                )
            }
        }
        _ if active || hovered => (t.invert_bg, Stroke::NONE, t.invert_fg),
        _ => (t.bg, Stroke::new(metrics.hairline_w, t.line_strong), t.ink),
    };
    p.rect(
        rect,
        CornerRadius::same(metrics.control_radius),
        fill,
        stroke,
        egui::StrokeKind::Inside,
    );
    let m = rect.center();
    match glyph {
        Glyph::Close => paint_line_glyph(p, glyph, m, (size * 0.15).clamp(3.5, 4.5), Stroke::new(1.5, c)),
        Glyph::Minimize => paint_line_glyph(p, glyph, m, (size * 0.18).clamp(4.0, 5.0), Stroke::new(1.5, c)),
        Glyph::Clone | Glyph::Expand => paint_box_glyph(p, glyph, m, size, c),
        _ => paint_font_glyph(
            p,
            glyph,
            m,
            icon_size.unwrap_or_else(|| (size * 0.48).clamp(12.0, 15.0)),
            c,
        ),
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
    drag_handle_custom(ui, theme::control_height(ui), tip)
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
        p.rect_filled(rect, CornerRadius::same(t.metrics().control_radius), t.sunken);
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

/// App wordmark in the display font, vertically centered in a container of the given height.
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

/// A compact checkbox that does not inflate to the skin's control height.
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

/// Height of the header search: it matches the analytics/settings glyph buttons next to it.
pub const HEADER_SEARCH_H: f32 = 34.0;

/// The TextEdit id of the header search. It is stable and explicit so the field keeps focus
/// across frames and so the header can clear it on Escape before the edit sees the key.
pub fn header_search_id() -> egui::Id {
    egui::Id::unique("header-rules-search")
}

/// A catalog search input in the shared visual language: an embedded magnifying glass, a field
/// radius from the active skin metrics, an integrated clear button, a background-free outlined
/// field and subtle focus highlighting. It fills the available width.
pub fn catalog_search_box(ui: &mut Ui, query: &mut String, hint_text: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let search_w = ui.available_width().max(60.0);
    let field_h = t.metrics().control_h.max(24.0);
    search_field(
        ui,
        query,
        hint_text,
        search_w,
        field_h,
        t.field_corner(),
        None,
    )
}

/// The same search field with an explicit width, for toolbars that must not stretch. The height
/// always comes from the active skin, like every other control.
pub fn catalog_search_box_sized(
    ui: &mut Ui,
    query: &mut String,
    hint_text: &str,
    search_w: f32,
) -> Response {
    let t = Tokens::get(ui.ctx());
    let field_h = t.metrics().control_h.max(24.0);
    search_field(
        ui,
        query,
        hint_text,
        search_w,
        field_h,
        t.field_corner(),
        None,
    )
}

/// The same search field with an explicit corner radius, so a catalog whose cards keep one radius
/// across both skins can make the field above the grid share it. Fills the available width.
pub fn catalog_search_box_with_corner(
    ui: &mut Ui,
    query: &mut String,
    hint_text: &str,
    corner: egui::CornerRadius,
) -> Response {
    let t = Tokens::get(ui.ctx());
    let search_w = ui.available_width().max(60.0);
    let field_h = t.metrics().control_h.max(24.0);
    search_field(ui, query, hint_text, search_w, field_h, corner, None)
}

/// The header rules search: the shared search field without a background, with an explicit width.
/// Escape is handled by the header, which knows whether the global results panel is open and can
/// close it before clearing the query.
pub fn header_search_box(ui: &mut Ui, query: &mut String, hint_text: &str, search_w: f32) -> Response {
    let corner = Tokens::get(ui.ctx()).field_corner();
    search_field(
        ui,
        query,
        hint_text,
        search_w,
        HEADER_SEARCH_H,
        corner,
        Some(header_search_id()),
    )
}

/// Shared body of the search field variants: outline, icon, text edit and clear button.
fn search_field(
    ui: &mut Ui,
    query: &mut String,
    hint_text: &str,
    search_w: f32,
    field_h: f32,
    corner: egui::CornerRadius,
    explicit_id: Option<egui::Id>,
) -> Response {
    let t = Tokens::get(ui.ctx());
    let search_id = explicit_id.unwrap_or_else(|| ui.scope_id().with("catalog_search_box_input"));
    let is_focused = ui.ctx().memory(|m| m.has_focus(search_id));
    let (box_rect, box_resp) = ui.allocate_exact_size(vec2(search_w, field_h), Sense::click());
    if box_resp.clicked() {
        ui.ctx().memory_mut(|m| m.request_focus(search_id));
    }

    let is_hovered = box_resp.hovered();
    // Every search field is background-free: a default-weight outline is the only chrome. The
    // same hairline answers the pointer and the accent answers focus, so no field grows a
    // thicker ring or a fill in either skin.
    let border = if is_focused {
        Stroke::new(t.metrics().hairline_w, t.accent_ink)
    } else if is_hovered {
        Stroke::new(t.metrics().hairline_w, t.line_strong)
    } else {
        t.hairline()
    };
    ui.painter()
        .rect_stroke(box_rect, corner, border, egui::StrokeKind::Inside);

    // Magnifying glass icon on left, perfectly centered vertically
    let icon_pos = pos2(box_rect.left() + 11.0, box_rect.center().y);
    let icon_color = if is_focused { t.accent_ink } else { t.muted };
    let icon_size = 15.0;
    ui.painter().text(
        icon_pos,
        egui::Align2::LEFT_CENTER,
        egui_phosphor::regular::MAGNIFYING_GLASS,
        egui::FontId::new(icon_size, theme::phosphor()),
        icon_color,
    );

    let has_query = !query.is_empty();
    let clear_w = if has_query { 28.0 } else { 10.0 };

    // The text edit lives in a child layout that centres its row vertically. The widget itself
    // keeps its natural line height, so no padding guess can push the text to the bottom edge.
    let text_rect = Rect::from_min_max(
        pos2(box_rect.left() + 32.0, box_rect.top()),
        pos2(box_rect.right() - clear_w, box_rect.bottom()),
    );
    let mut edit_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    // The style sets a control-height floor on every widget. A search field shorter than that
    // floor would otherwise contain a full-height edit whose text row sits below the magnifying
    // glass, so the child layout gets the field's own row height.
    edit_ui.spacing_mut().interact_size.y = field_h;
    // The edit keeps its natural row height and the child layout centres that row in the field.
    // Forcing the widget to the full field height with `add_sized` pinned the text several pixels
    // below the magnifying glass in the short header field.
    let edit_resp = edit_ui.add(
        egui::TextEdit::singleline(query)
            .id(search_id)
            .hint_text(
                egui::RichText::new(hint_text.to_uppercase())
                    .family(theme::medium())
                    .size(11.0)
                    .color(t.faint),
            )
            .font(egui::FontId::new(11.0, theme::medium()))
            .text_color(t.ink)
            .frame(egui::Frame::NONE)
            .margin(vec2(0.0, 0.0))
            .desired_width(text_rect.width().max(10.0))
            .align(egui::Align2::LEFT_CENTER),
    );

    // Clear button on the right
    if has_query {
        let clear_rect = Rect::from_center_size(
            pos2(box_rect.right() - 15.0, box_rect.center().y),
            vec2(18.0, 18.0),
        );
        let clear_resp = ui.interact(clear_rect, search_id.with("clear"), Sense::click());
        let clear_hovered = clear_resp.hovered();
        if clear_hovered {
            ui.painter().rect_filled(clear_rect, t.chip_corner(), t.sunken);
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

    /// The search hint must sit on the vertical centre of the field box, in any container.
    #[test]
    fn search_hint_is_vertically_centred_in_the_field() {
        let ctx = egui::Context::default();
        crate::gui::theme::install_fonts(&ctx);
        crate::gui::theme::apply_theme(
            &ctx,
            crate::config::AppTheme::Dark,
            crate::config::UiSkin::Modern,
            None,
        );

        let mut query = String::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(420.0, 120.0),
                )),
                ..Default::default()
            },
            |ui| {
                ui.add_space(40.0);
                let _ = super::catalog_search_box_sized(ui, &mut query, "search actions", 360.0);
            },
        );
        output.textures_delta.clear();

        let field_centre_y = 40.0 + 16.0;
        let mut seen = false;
        for clipped in &output.shapes {
            if let egui::Shape::Text(text) = &clipped.shape {
                if text.galley.text().to_lowercase().contains("search actions") {
                    let centre = text.pos.y + text.galley.size().y * 0.5;
                    assert!(
                        (centre - field_centre_y).abs() <= 3.0,
                        "hint centre {centre} is not the field centre {field_centre_y}"
                    );
                    seen = true;
                }
            }
        }
        assert!(seen, "the hint must be rendered");
    }

    /// The header field is shorter than a control row, but its text must still share the exact
    /// centre line with the magnifying glass: the hint and the typed text are both checked.
    #[test]
    fn header_search_text_is_vertically_centred() {
        let ctx = egui::Context::default();
        crate::gui::theme::install_fonts(&ctx);
        crate::gui::theme::apply_theme(
            &ctx,
            crate::config::AppTheme::Dark,
            crate::config::UiSkin::Modern,
            None,
        );

        let render = |query: &mut String| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(500.0, 80.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let _ = super::header_search_box(ui, query, "search rules", 420.0);
                },
            );
            output.textures_delta.clear();
            output
        };

        for query in [String::new(), "macro".to_string()] {
            let mut query = query;
            let output = render(&mut query);
            let field_centre_y = super::HEADER_SEARCH_H * 0.5;
            let mut seen = false;
            for clipped in &output.shapes {
                if let egui::Shape::Text(text) = &clipped.shape {
                    let label = text.galley.text().to_lowercase();
                    if label.contains("search rules") || label.contains("macro") {
                        let centre = text.pos.y + text.galley.size().y * 0.5;
                        assert!(
                            (centre - field_centre_y).abs() <= 2.0,
                            "text centre {centre} is not the field centre {field_centre_y} for {query:?}"
                        );
                        seen = true;
                    }
                }
            }
            assert!(seen, "the text must be rendered for {query:?}");
        }
    }

    /// The header glyph has a larger mark than a dense toolbar button; the mark must still sit on
    /// the button's centre, not drift with the size.
    #[test]
    fn sized_glyph_icon_stays_on_the_button_centre() {
        let ctx = egui::Context::default();
        crate::gui::theme::install_fonts(&ctx);
        crate::gui::theme::apply_theme(
            &ctx,
            crate::config::AppTheme::Dark,
            crate::config::UiSkin::Modern,
            None,
        );
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(120.0, 120.0))),
                ..Default::default()
            },
            |ui| {
                let _ = super::glyph_button_outlined(ui, Glyph::Settings, 34.0, 18.0, "settings");
            },
        );
        output.textures_delta.clear();

        let mut seen = false;
        for clipped in &output.shapes {
            if let egui::Shape::Text(text) = &clipped.shape {
                let centre = text.pos + text.galley.size() * 0.5;
                assert!(
                    (centre.x - 17.0).abs() <= 1.0 && (centre.y - 17.0).abs() <= 1.0,
                    "the icon centre {centre:?} is not the button centre"
                );
                seen = true;
            }
        }
        assert!(seen, "the icon must be painted");
    }

    #[test]
    fn modern_primary_reacts_to_hover_in_both_palettes() {
        for theme in [
            crate::config::AppTheme::Dark,
            crate::config::AppTheme::Light,
        ] {
            let t = Tokens::of(theme, UiSkin::Modern);
            let rest = kind_paint(Kind::Primary, &t, 0.0, false);
            let hovered = kind_paint(Kind::Primary, &t, 1.0, false);
            assert_ne!(
                rest.fill, hovered.fill,
                "the primary button must show hover feedback in {theme:?}"
            );
            assert_eq!(rest.ink, hovered.ink, "the label colour stays readable");
        }
    }

    #[test]
    fn kind_paint_resolves_every_variant_for_both_button_sizes() {
        let t = Tokens::DARK;

        let primary = kind_paint(Kind::Primary, &t, 0.0, false);
        assert_eq!(primary.fill, t.invert_bg);
        assert_eq!(primary.ink, t.invert_fg);
        assert_eq!(primary.stroke, Stroke::NONE);
        // The compact helper used to paint Primary with the accent colour instead.
        let primary_compact = kind_paint(Kind::Primary, &t, 0.0, true);
        assert_eq!(primary_compact.fill, t.invert_bg);
        assert_eq!(primary_compact.ink, t.invert_fg);
        assert_ne!(kind_paint(Kind::Primary, &t, 1.0, false).fill, t.invert_bg);

        let secondary = kind_paint(Kind::Secondary, &t, 0.0, false);
        assert_eq!(secondary.fill, t.bg);
        assert_eq!(secondary.ink, t.ink);
        assert_eq!(secondary.stroke.width, 1.0);
        assert_eq!(kind_paint(Kind::Secondary, &t, 0.0, true).fill, t.sunken);
        let secondary_hovered = kind_paint(Kind::Secondary, &t, 1.0, false);
        assert_eq!(secondary_hovered.fill, t.invert_bg);
        assert_eq!(secondary_hovered.ink, t.invert_fg);

        let danger = kind_paint(Kind::Danger, &t, 0.0, true);
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
            Glyph::Analytics,
            Glyph::Settings,
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
