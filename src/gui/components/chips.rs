//! Shared tag-pill primitive: one geometry, typography and paint pass for every rule-tag
//! pill in the GUI.
//!
//! The rules card, the inspector editor and the inspector "existing tags" picker used to
//! repeat the same pill structure at different sizes. [`TagPillStyle`] holds the differences;
//! [`tag_pill_width`] measures and [`paint_tag_pill`] paints the shared chrome (background
//! fill + stroke, then the icon, then the text). Interaction stays with each site: the card
//! rows register their hover ids and own the overflow `+N` badge, the inspector editor paints
//! its remove button after the pill, and the picker resolves its hover chrome before calling
//! the painter.

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, FontId, Galley, Rect, Stroke, Ui, pos2, vec2};

use crate::gui::theme;
use crate::models::RuleTag;

/// The font family a pill lays its text out with.
#[derive(Clone, Copy)]
enum TagFont {
    /// Geist Mono, used by the rules-card pills.
    Mono,
    /// Inter Medium, used by the inspector pills.
    Medium,
}

impl TagFont {
    fn family(self) -> egui::FontFamily {
        match self {
            Self::Mono => theme::mono(),
            Self::Medium => theme::medium(),
        }
    }
}

/// How a pill measures, positions and paints its icon.
#[derive(Clone, Copy)]
enum TagIconLayout {
    /// The glyph's own advance, followed by `gap` before the text (rules card).
    Galley { gap: f32 },
    /// A fixed-width slot with the glyph centred, then a fixed advance to the text
    /// (inspector editor and picker). `measure_width` is the slot's contribution to the
    /// pill width.
    Slot {
        measure_width: f32,
        glyph_width: f32,
        advance: f32,
    },
}

/// Background chrome for one pill state.
#[derive(Clone, Copy)]
struct TagPillChrome {
    fill_gamma: f32,
    stroke_gamma: f32,
    stroke_width: f32,
}

/// Geometry, typography and chrome for one kind of tag pill.
///
/// The three constant instances are the only styles; views never construct one. The numbers
/// are pixel geometry taken from the sites this module replaced, not preferences.
pub(crate) struct TagPillStyle {
    /// Pill height in points; callers allocate or place the rect with it.
    pub(crate) height: f32,
    corner: u8,
    pad_left: f32,
    pad_right: f32,
    min_width: f32,
    text_size: f32,
    text_font: TagFont,
    icon_size: f32,
    icon_layout: TagIconLayout,
    chrome: TagPillChrome,
    hover_chrome: Option<TagPillChrome>,
}

impl TagPillStyle {
    /// Rules-list card pill: 17 px tall, corner 3, mono 9.5 text, phosphor 10 galley icon.
    pub(crate) const CARD: Self = Self {
        height: 17.0,
        corner: 3,
        pad_left: 5.0,
        pad_right: 5.0,
        min_width: 18.0,
        text_size: 9.5,
        text_font: TagFont::Mono,
        icon_size: 10.0,
        icon_layout: TagIconLayout::Galley { gap: 3.5 },
        chrome: TagPillChrome {
            fill_gamma: 0.18,
            stroke_gamma: 0.50,
            stroke_width: 0.8,
        },
        hover_chrome: None,
    };

    /// Inspector editor pill: 20 px tall, corner 10, 10 medium text, 14 px icon slot at 11 px.
    pub(crate) const INSPECTOR_EDITOR: Self = Self {
        height: 20.0,
        corner: 10,
        pad_left: 8.0,
        pad_right: 16.0,
        min_width: 0.0,
        text_size: 10.0,
        text_font: TagFont::Medium,
        icon_size: 11.0,
        icon_layout: TagIconLayout::Slot {
            measure_width: 16.0,
            glyph_width: 14.0,
            advance: 15.0,
        },
        chrome: TagPillChrome {
            fill_gamma: 0.18,
            stroke_gamma: 0.45,
            stroke_width: 1.0,
        },
        hover_chrome: None,
    };

    /// Inspector picker pill: 18 px tall, corner 9, 9.5 medium text, 12 px icon slot at 10 px.
    pub(crate) const INSPECTOR_PICKER: Self = Self {
        height: 18.0,
        corner: 9,
        pad_left: 6.0,
        pad_right: 8.0,
        min_width: 0.0,
        text_size: 9.5,
        text_font: TagFont::Medium,
        icon_size: 10.0,
        icon_layout: TagIconLayout::Slot {
            measure_width: 14.0,
            glyph_width: 12.0,
            advance: 13.0,
        },
        chrome: TagPillChrome {
            fill_gamma: 0.18,
            stroke_gamma: 0.50,
            stroke_width: 1.0,
        },
        hover_chrome: Some(TagPillChrome {
            fill_gamma: 0.35,
            stroke_gamma: 1.0,
            stroke_width: 1.0,
        }),
    };
}

/// The tag's icon when it has one to paint.
fn tag_icon(tag: &RuleTag) -> Option<&str> {
    tag.icon.as_deref().filter(|icon| !icon.is_empty())
}

/// Lays the tag name out verbatim in the style's font. `color` only feeds layout; the painted
/// colour is supplied separately by [`paint_tag_pill`]. The case is never transformed here: the
/// inspector preview must show the tag exactly as the rules card does.
fn layout_text(ui: &Ui, tag: &RuleTag, style: &TagPillStyle, color: Color32) -> Arc<Galley> {
    ui.painter().layout_no_wrap(
        tag.name.clone(),
        FontId::new(style.text_size, style.text_font.family()),
        color,
    )
}

fn layout_icon(ui: &Ui, icon: &str, style: &TagPillStyle, color: Color32) -> Arc<Galley> {
    ui.painter().layout_no_wrap(
        icon.to_owned(),
        FontId::new(style.icon_size, theme::phosphor()),
        color,
    )
}

/// Width of one tag pill, measured with the same fonts the paint pass uses.
pub(crate) fn tag_pill_width(ui: &Ui, tag: &RuleTag, style: &TagPillStyle) -> f32 {
    let text_w = layout_text(ui, tag, style, Color32::WHITE).size().x;
    let width = match style.icon_layout {
        // The rules card lets the icon glyph's own advance set the width and adds the gap.
        TagIconLayout::Galley { gap } => {
            let (icon_w, gap) = match tag_icon(tag) {
                Some(icon) => (layout_icon(ui, icon, style, Color32::WHITE).size().x, gap),
                None => (0.0, 0.0),
            };
            style.pad_left + icon_w + gap + text_w + style.pad_right
        }
        // The inspector reserves a fixed slot for the icon; the slot is part of the width.
        TagIconLayout::Slot { measure_width, .. } => {
            let icon_w = if tag_icon(tag).is_some() {
                measure_width
            } else {
                0.0
            };
            text_w + icon_w + (style.pad_left + style.pad_right)
        }
    };
    width.max(style.min_width)
}

/// Paints one tag pill: background fill + stroke, then the icon, then the label.
///
/// `color` is the tag colour; the style derives fill and stroke from it. `text_color` and
/// `icon_color` are the painted colours, and `hovered` selects the style's hover chrome
/// (only the inspector picker has one). No interaction is registered here: each site keeps
/// its own ids, hover behaviour and trailing widgets.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_tag_pill(
    ui: &Ui,
    rect: Rect,
    tag: &RuleTag,
    style: &TagPillStyle,
    color: Color32,
    text_color: Color32,
    icon_color: Color32,
    hovered: bool,
) {
    let chrome = match (hovered, style.hover_chrome) {
        (true, Some(hover)) => hover,
        _ => style.chrome,
    };
    let t = crate::gui::theme::Tokens::get(ui.ctx());
    let corner = if t.metrics().skin == crate::config::UiSkin::Modern {
        t.metrics().chip_radius
    } else {
        style.corner
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(corner),
        color.gamma_multiply(chrome.fill_gamma),
        Stroke::new(chrome.stroke_width, color.gamma_multiply(chrome.stroke_gamma)),
        egui::StrokeKind::Inside,
    );

    let mut text_x = rect.left() + style.pad_left;
    if let Some(icon) = tag_icon(tag) {
        match style.icon_layout {
            TagIconLayout::Galley { gap } => {
                let galley = layout_icon(ui, icon, style, color);
                let icon_w = galley.size().x;
                painter.galley(
                    pos2(text_x, rect.center().y - galley.size().y / 2.0),
                    galley,
                    icon_color,
                );
                text_x += icon_w + gap;
            }
            TagIconLayout::Slot {
                glyph_width,
                advance,
                ..
            } => {
                let slot = Rect::from_min_size(
                    pos2(text_x, rect.top()),
                    vec2(glyph_width, rect.height()),
                );
                painter.text(
                    slot.center(),
                    Align2::CENTER_CENTER,
                    icon,
                    FontId::new(style.icon_size, theme::phosphor()),
                    icon_color,
                );
                text_x += advance;
            }
        }
    }

    let galley = layout_text(ui, tag, style, color);
    painter.galley(
        pos2(text_x, rect.center().y - galley.size().y / 2.0),
        galley,
        text_color,
    );
}
