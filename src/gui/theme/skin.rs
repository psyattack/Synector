//! Skin metrics: geometry, density, strokes, elevation and motion of one visual generation.
//!
//! A view reads the resolved [`Tokens`](super::Tokens) and its metrics, so the classic
//! generation (the original studio identity: hard edges, hairline rules, square chrome) and the
//! modern one share one code path. [`Metrics::of`] is the only mapping from [`UiSkin`] to
//! values; a component that genuinely composes an element differently per generation (a toggle, a
//! filter cell, a badge) reads `Metrics::skin` for that one branch, never to pick a colour.

use crate::config::UiSkin;

/// Height of buttons, inputs and filter cells in the classic skin.
pub const CONTROL_H: f32 = 34.0;

/// Height of the top navigation/title bar in the classic skin.
pub const HEADER_H: f32 = 48.0;

/// Height of the bottom telemetry/status bar in the classic skin.
pub const FOOTER_H: f32 = 34.0;

/// The precision `.hud-label`: 11 px Geist Mono 500, uppercase, 0.15 em letter-spacing.
pub const HUD_SIZE: f32 = 11.0;
pub const HUD_SPACING: f32 = HUD_SIZE * 0.15;

/// Button labels: 11 px Geist Mono 700, uppercase, 0.12 em letter-spacing.
pub const BUTTON_SPACING: f32 = HUD_SIZE * 0.12;

/// Elevation of a floating surface. Alpha is per palette darkness because a shadow that reads
/// on a dark page disappears on a light one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shadow {
    pub dark_alpha: u8,
    pub light_alpha: u8,
    pub blur: u8,
    pub offset_y: i8,
}

/// Motion timings of one skin, in seconds. The OS can still switch motion off entirely; the
/// durations are the intent, not the guarantee.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    /// Hover and press feedback.
    pub fast: f32,
    /// Standard transitions.
    pub base: f32,
    /// The inspector drawer.
    pub drawer: f32,
    /// A dialog opening.
    pub modal_open: f32,
    /// A dialog closing.
    pub modal_close: f32,
}

/// Geometry, density, stroke, elevation and motion metrics of one skin.
///
/// Every field has a single owner: the skin. A view or a component that needs a radius, a row
/// height, a hairline width or a transition time reads it here instead of hard-coding a number,
/// which is what lets a second generation ship without a second view tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Which generation these metrics describe; components use it only where the two
    /// generations genuinely compose an element differently (a toggle, a filter cell), never to
    /// pick a colour.
    pub skin: UiSkin,
    /// Outer card and preview-tile radius.
    pub card_radius: u8,
    /// Radius of buttons and framed controls.
    pub control_radius: u8,
    /// Radius of search fields and text-entry wells.
    pub field_radius: u8,
    /// Radius of tag pills and badges.
    pub chip_radius: u8,
    /// Radius of small inset blocks: code blocks, doc cards, nested panels.
    pub inset_radius: u8,
    /// Radius of dialog cards.
    pub modal_radius: u8,
    /// Radius of popups, menus and tooltips.
    pub popup_radius: u8,
    /// Height of one control row.
    pub control_h: f32,
    /// Height of the top navigation/title bar.
    pub header_h: f32,
    /// Height of the bottom telemetry/status bar.
    pub footer_h: f32,
    /// Width of a toggle switch.
    pub toggle_w: f32,
    /// Height of a toggle switch.
    pub toggle_h: f32,
    /// Width of a hairline divider and border.
    pub hairline_w: f32,
    /// Alpha of a raised surface over the backdrop; 255 is fully opaque.
    pub surface_alpha: u8,
    /// Elevation of floating surfaces.
    pub shadow: Shadow,
    /// Transition timings.
    pub motion: Motion,
}

impl Metrics {
    /// The original studio generation: hard edges, hairline rules, dense rows.
    pub const CLASSIC: Self = Self {
        skin: UiSkin::Classic,
        card_radius: 0,
        control_radius: 0,
        field_radius: 6,
        chip_radius: 3,
        inset_radius: 2,
        modal_radius: 2,
        popup_radius: 0,
        control_h: CONTROL_H,
        header_h: HEADER_H,
        footer_h: FOOTER_H,
        toggle_w: 38.0,
        toggle_h: 20.0,
        hairline_w: 1.0,
        surface_alpha: 255,
        shadow: Shadow {
            dark_alpha: 140,
            light_alpha: 36,
            blur: 24,
            offset_y: 8,
        },
        motion: Motion {
            fast: 0.10,
            base: 0.15,
            drawer: 0.22,
            modal_open: 0.24,
            modal_close: 0.16,
        },
    };

    /// The modern generation: rounded layered surfaces, a sans-serif voice, softer elevation.
    pub const MODERN: Self = Self {
        skin: UiSkin::Modern,
        card_radius: 14,
        control_radius: 10,
        field_radius: 12,
        chip_radius: 8,
        inset_radius: 10,
        modal_radius: 16,
        popup_radius: 12,
        control_h: 38.0,
        header_h: HEADER_H,
        footer_h: FOOTER_H,
        toggle_w: 40.0,
        toggle_h: 22.0,
        hairline_w: 1.0,
        surface_alpha: 224,
        shadow: Shadow {
            dark_alpha: 110,
            light_alpha: 26,
            blur: 40,
            offset_y: 12,
        },
        motion: Motion {
            fast: 0.12,
            base: 0.18,
            drawer: 0.26,
            modal_open: 0.26,
            modal_close: 0.18,
        },
    };

    pub fn of(skin: UiSkin) -> Self {
        match skin {
            UiSkin::Classic => Self::CLASSIC,
            UiSkin::Modern => Self::MODERN,
        }
    }
}
