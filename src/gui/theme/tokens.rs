//! Colour palettes and the resolved per-frame design tokens.
//!
//! A [`Palette`] owns the colour roles of one theme; [`Metrics`] (in `skin`) owns the geometry,
//! density and stroke metrics of the active visual generation. [`Tokens`] is the single value a
//! view reads — `t.bg`, `t.hairline()`, `t.metrics()` — so a view never branches on the theme or
//! on the skin. [`Tokens::get`] resolves both axes from egui's temporary data every frame.

use std::ops::Deref;

use egui::{Color32, Stroke};

use crate::config::{AppTheme, UiSkin};

use super::skin::Metrics;

const fn ink_alpha(rgb: u32, alpha: u8) -> Color32 {
    let [_, r, g, b] = rgb.to_be_bytes();
    Color32::from_rgba_unmultiplied_const(r, g, b, alpha)
}

const fn hex(rgb: u32) -> Color32 {
    let [_, r, g, b] = rgb.to_be_bytes();
    Color32::from_rgb(r, g, b)
}

/// Blends `b` into `a` by `amount`.
fn mix(a: Color32, b: Color32, amount: f32) -> Color32 {
    let f = amount.clamp(0.0, 1.0);
    let channel = |from: u8, to: u8| {
        (from as f32 + (to as f32 - from as f32) * f)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}

fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// The colour roles of one theme.
///
/// Roles are semantic, not literal: a view asks for `sunken` or `line_strong`, never for a hex
/// value, so a palette can be retuned without touching any view.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// Page and card surface.
    pub bg: Color32,
    /// Recessed wells, badges and row hovers.
    pub sunken: Color32,
    /// Standard body text.
    pub ink: Color32,
    /// High-emphasis headings and labels.
    pub ink_strong: Color32,
    /// Secondary text.
    pub muted: Color32,
    /// Faint tertiary text, separators, indices.
    pub faint: Color32,
    /// Subtle hairlines (15% opacity).
    pub line: Color32,
    /// Emphasized hairlines / active outlines (40% opacity).
    pub line_strong: Color32,
    /// Primary accent (for surfaces): a neutral ink tone, so a call to action never becomes a
    /// slab of saturated colour.
    pub accent: Color32,
    /// Primary brand accent (for text and icons).
    pub accent_ink: Color32,
    /// Inversion fill (solid white in dark mode).
    pub invert_bg: Color32,
    /// Inversion ink (near black in dark mode).
    pub invert_fg: Color32,
    /// Status emerald green.
    pub good: Color32,
    /// Status amber warning.
    pub warn: Color32,
    /// Selected-text background; tinted by a user accent.
    pub select_bg: Color32,
    /// Status crimson danger.
    pub bad: Color32,
}

impl Palette {
    pub const DARK: Self = Self {
        dark: true,
        bg: hex(0x121316),
        sunken: hex(0x060607),
        ink: hex(0xf2f1ee),
        ink_strong: hex(0xffffff),
        muted: ink_alpha(0xf2f1ee, 153),
        faint: ink_alpha(0xf2f1ee, 97),
        line: ink_alpha(0xf2f1ee, 38),
        line_strong: ink_alpha(0xf2f1ee, 102),
        accent: hex(0x2d81ff),
        accent_ink: hex(0x74aaff),
        invert_bg: hex(0xffffff),
        invert_fg: hex(0x0b0b0c),
        good: hex(0x3fd6b8),
        warn: hex(0xffc14d),
        bad: hex(0xff6259),
        select_bg: hex(0x33343a),
    };

    pub const LIGHT: Self = Self {
        dark: false,
        bg: hex(0xf2f1ee),
        sunken: hex(0xe9e8e4),
        ink: hex(0x101014),
        ink_strong: hex(0x000000),
        muted: ink_alpha(0x101014, 158),
        faint: ink_alpha(0x101014, 107),
        line: ink_alpha(0x101014, 41),
        line_strong: ink_alpha(0x101014, 102),
        accent: hex(0x2d81ff),
        accent_ink: hex(0x1659c4),
        invert_bg: hex(0x101014),
        invert_fg: hex(0xf2f1ee),
        good: hex(0x0a7563),
        warn: hex(0x8a5800),
        bad: hex(0xb3261e),
        select_bg: hex(0xd4d2cc),
    };

    /// The palette for a configured [`AppTheme`]; the single derivation used by
    /// [`Tokens::of`] and by the settings previews so the two cannot drift apart.
    pub fn of(theme: AppTheme) -> Self {
        match theme {
            AppTheme::Dark => Self::DARK,
            AppTheme::Light => Self::LIGHT,
        }
    }

    /// Applies a user accent on top of this palette, or leaves it untouched.
    pub fn with_optional_accent(self, accent: Option<[u8; 3]>) -> Self {
        match accent {
            Some(rgb) => self.with_accent(rgb),
            None => self,
        }
    }

    /// Re-tints the accent roles of a base palette. [`Palette::accent_ink`] is derived from the
    /// chosen colour so accent text stays readable on the page in either palette, and the
    /// selection background takes on the accent's hue so a text selection follows the choice.
    /// Call it once on a base palette: re-applying it would tint an already tinted selection.
    pub fn with_accent(mut self, [r, g, b]: [u8; 3]) -> Self {
        self.accent = Color32::from_rgb(r, g, b);
        let target = if self.dark { Color32::WHITE } else { Color32::BLACK };
        self.accent_ink = mix(self.accent, target, if self.dark { 0.42 } else { 0.30 });
        self.select_bg = mix(self.select_bg, self.accent, 0.35);
        self
    }

    /// Blends two palettes for the theme cross-fade. The `dark` flag is decided by the target:
    /// the branch that depends on it (shadow alpha, surface lift) should already be the final
    /// one while the colours are still travelling.
    fn lerp(from: Self, to: Self, t: f32) -> Self {
        let blend = |a: Color32, b: Color32| a.lerp_to_gamma(b, t);
        Self {
            dark: to.dark,
            bg: blend(from.bg, to.bg),
            sunken: blend(from.sunken, to.sunken),
            ink: blend(from.ink, to.ink),
            ink_strong: blend(from.ink_strong, to.ink_strong),
            muted: blend(from.muted, to.muted),
            faint: blend(from.faint, to.faint),
            line: blend(from.line, to.line),
            line_strong: blend(from.line_strong, to.line_strong),
            accent: blend(from.accent, to.accent),
            accent_ink: blend(from.accent_ink, to.accent_ink),
            invert_bg: blend(from.invert_bg, to.invert_bg),
            invert_fg: blend(from.invert_fg, to.invert_fg),
            select_bg: blend(from.select_bg, to.select_bg),
            good: blend(from.good, to.good),
            warn: blend(from.warn, to.warn),
            bad: blend(from.bad, to.bad),
        }
    }
}

/// A running palette cross-fade, stored in egui's temporary data.
#[derive(Clone, Copy, Debug)]
struct PaletteFade {
    from: Palette,
    started_at: f64,
    duration: f32,
}

impl Default for PaletteFade {
    /// Required by egui's temporary-data API; a zero duration is never treated as running.
    fn default() -> Self {
        Self {
            from: Palette::DARK,
            started_at: 0.0,
            duration: 0.0,
        }
    }
}

fn fade_id() -> egui::Id {
    egui::Id::unique("palette_fade")
}

/// Identity of the user-chosen accent stored in egui's temporary data.
pub(crate) fn accent_id() -> egui::Id {
    egui::Id::unique("app_accent_rgb")
}

/// Parses an accent colour from `#RRGGBB`; anything else is ignored.
pub fn parse_accent(value: Option<&str>) -> Option<[u8; 3]> {
    value.and_then(crate::models::parse_hex_color)
}

/// Starts (or replaces) the cross-fade from `from` to `to`. A zero duration removes it.
pub(crate) fn begin_palette_fade(
    ctx: &egui::Context,
    from: Palette,
    to: Palette,
    duration: f32,
) {
    let started_at = ctx.input(|input| input.time);
    ctx.data_mut(|data| {
        if duration <= 0.0 || from == to {
            data.remove_temp::<PaletteFade>(fade_id());
        } else {
            data.insert_temp(
                fade_id(),
                PaletteFade {
                    from,
                    started_at,
                    duration,
                },
            );
        }
    });
}

/// The palette a view must read this frame: the target palette, or its blend with the previous
/// one while a fade is running.
fn fading_palette(ctx: &egui::Context, target: Palette) -> Palette {
    let Some(fade) = ctx.data(|data| data.get_temp::<PaletteFade>(fade_id())) else {
        return target;
    };
    let now = ctx.input(|input| input.time);
    let progress = ((now - fade.started_at) as f32 / fade.duration).clamp(0.0, 1.0);
    if progress >= 1.0 {
        return target;
    }
    Palette::lerp(fade.from, target, egui::emath::easing::cubic_out(progress))
}

/// State of the palette cross-fade at the start of a frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FadeStatus {
    /// No fade is running.
    Idle,
    /// A fade is running; the palette a view reads is the blend.
    Running,
    /// The fade ended on this frame and was dropped: the target palette is now the only one.
    Settled,
}

/// Advances the running fade and reports its state. A finished fade is removed here, which is
/// the only moment the caller can apply the final visuals before the blend is forgotten.
pub(crate) fn palette_fade_status(ctx: &egui::Context, target: Palette) -> FadeStatus {
    let Some(fade) = ctx.data(|data| data.get_temp::<PaletteFade>(fade_id())) else {
        return FadeStatus::Idle;
    };
    let now = ctx.input(|input| input.time);
    let done = now - fade.started_at >= f64::from(fade.duration) || fade.from == target;
    if done {
        ctx.data_mut(|data| data.remove_temp::<PaletteFade>(fade_id()));
        FadeStatus::Settled
    } else {
        FadeStatus::Running
    }
}

/// Ends a running fade immediately. Tests have no frame clock to advance, so they settle the
/// palette explicitly before asserting on the target colours.
#[cfg(test)]
pub(crate) fn finish_palette_fade(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove_temp::<PaletteFade>(fade_id()));
}

/// The resolved design tokens of one frame: the active palette plus the active skin metrics.
///
/// `Tokens` dereferences to its [`Palette`], so existing `t.bg` / `t.ink_strong` reads keep
/// working while geometry comes from `t.metrics()`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    palette: Palette,
    metrics: Metrics,
}

impl Tokens {
    pub fn of(theme: AppTheme, skin: UiSkin) -> Self {
        Self {
            palette: Palette::of(theme),
            metrics: Metrics::of(skin),
        }
    }

    /// The tokens (palette and skin) the context is currently rendering with. During a palette
    /// cross-fade the colours are the blend; the skin metrics are always the target's, and a
    /// user-chosen accent is applied last so it survives the fade.
    pub fn get(ctx: &egui::Context) -> Self {
        let theme = ctx
            .data(|d| d.get_temp::<AppTheme>(egui::Id::unique("app_theme")))
            .unwrap_or(AppTheme::Dark);
        let skin = ctx
            .data(|d| d.get_temp::<UiSkin>(egui::Id::unique("app_skin")))
            .unwrap_or_default();
        let mut palette = fading_palette(ctx, Palette::of(theme));
        if let Some(accent) = ctx.data(|d| d.get_temp::<Option<[u8; 3]>>(accent_id())).flatten() {
            palette = palette.with_accent(accent);
        }
        Self {
            palette,
            metrics: Metrics::of(skin),
        }
    }

    /// Applies a user-chosen accent on top of the palette, or leaves it untouched.
    pub fn with_accent(mut self, accent: Option<[u8; 3]>) -> Self {
        if let Some(rgb) = accent {
            self.palette = self.palette.with_accent(rgb);
        }
        self
    }

    /// Geometry, density and stroke metrics of the active skin.
    pub fn metrics(&self) -> Metrics {
        self.metrics
    }

    /// The colour roles of the active palette, for caches that depend on colours only.
    pub fn palette(&self) -> Palette {
        self.palette
    }

    /// Categorical chart series colour. The values are derived from the semantic roles, so both
    /// palettes and a user-chosen accent stay coherent and no chart hardcodes a hex value. The
    /// index wraps; charts pass stable category indices, never list positions.
    pub fn series(&self, index: usize) -> Color32 {
        let p = self.palette;
        // Every arm is an opaque colour derived from the semantic roles: a translucent text role
        // would let the card surface bleed through a chart mark, and the series would stop being
        // comparable across the two palettes.
        match index % 10 {
            0 => p.accent_ink,
            1 => p.good,
            2 => p.warn,
            3 => mix(p.good, p.accent_ink, 0.5),
            4 => mix(p.warn, p.bad, 0.5),
            5 => mix(p.accent_ink, p.ink_strong, 0.8),
            6 => mix(p.accent_ink, p.ink_strong, 0.45),
            7 => mix(p.good, p.warn, 0.5),
            8 => mix(p.bad, p.accent_ink, 0.5),
            _ => mix(p.good, p.bad, 0.55),
        }
    }

    /// The page surface a panel paints over the backdrop.
    pub fn surface(&self) -> Color32 {
        with_alpha(self.palette.bg, self.metrics.surface_alpha)
    }

    /// A surface lifted one step off the page: buttons, selected segments, floating rows.
    pub fn raised(&self) -> Color32 {
        let lift = if self.palette.dark { 0.10 } else { 0.045 };
        with_alpha(
            mix(self.palette.bg, self.palette.ink, lift),
            self.metrics.surface_alpha,
        )
    }

    /// A raised surface under the pointer: one more step off the page than [`Tokens::raised`].
    pub fn raised_hover(&self) -> Color32 {
        let lift = if self.palette.dark { 0.17 } else { 0.085 };
        with_alpha(
            mix(self.palette.bg, self.palette.ink, lift),
            self.metrics.surface_alpha,
        )
    }

    /// Translucent fill for a regular button: it lets the page wash through instead of
    /// painting an opaque grey slab, which is what reads as glass in the dark palette.
    pub fn glass_button(&self, hover: f32) -> Color32 {
        let base = if self.palette.dark { 12.0 } else { 9.0 };
        let extra = if self.palette.dark { 11.0 } else { 7.0 };
        let alpha = (base + extra * hover.clamp(0.0, 1.0)).round() as u8;
        Color32::from_rgba_unmultiplied(self.ink.r(), self.ink.g(), self.ink.b(), alpha)
    }

    /// A row or list hover: a barely-there lift, so hovering never paints a grey slab.
    pub fn hover_fill(&self) -> Color32 {
        let alpha = if self.palette.dark { 10 } else { 8 };
        Color32::from_rgba_unmultiplied(self.ink.r(), self.ink.g(), self.ink.b(), alpha)
    }

    /// The one-pixel inner highlight along the top edge of a lifted surface: it is what makes
    /// a dark panel read as glass instead of flat grey.
    pub fn surface_highlight(&self) -> Color32 {
        if self.palette.dark {
            Color32::from_white_alpha(18)
        } else {
            Color32::from_white_alpha(120)
        }
    }

    pub fn hairline(&self) -> Stroke {
        Stroke::new(self.metrics.hairline_w, self.line)
    }

    /// Border of a framed control and of a text-entry well at rest: the same material
    /// `apply_theme` gives the native widgets, exposed so a hand-painted field can match the
    /// input next to it.
    pub fn field_stroke(&self) -> Stroke {
        let color = if self.metrics.skin == UiSkin::Modern {
            self.line.lerp_to_gamma(self.line_strong, 0.55)
        } else {
            self.line
        };
        Stroke::new(self.metrics.hairline_w, color)
    }

    /// Corner radius of a card or an inset panel.
    pub fn card_corner(&self) -> egui::CornerRadius {
        egui::CornerRadius::same(self.metrics.card_radius)
    }

    /// Corner radius of a button, a segment or a framed control.
    pub fn control_corner(&self) -> egui::CornerRadius {
        egui::CornerRadius::same(self.metrics.control_radius)
    }

    /// Corner radius of a text field or a well that behaves like one.
    pub fn field_corner(&self) -> egui::CornerRadius {
        egui::CornerRadius::same(self.metrics.field_radius)
    }

    /// Corner radius of a tag pill or a badge.
    pub fn chip_corner(&self) -> egui::CornerRadius {
        egui::CornerRadius::same(self.metrics.chip_radius)
    }

    /// Corner radius of a small inset block: a code block, a doc card, a nested panel.
    pub fn inset_corner(&self) -> egui::CornerRadius {
        egui::CornerRadius::same(self.metrics.inset_radius)
    }

    /// Corner radius of a side panel that touches the window edge: square in the classic
    /// generation, rounded on its inner edge in the modern one.
    pub fn drawer_corner(&self) -> egui::CornerRadius {
        let r = self.metrics.card_radius;
        if r == 0 {
            egui::CornerRadius::ZERO
        } else {
            egui::CornerRadius {
                nw: r,
                ne: 0,
                sw: r,
                se: 0,
            }
        }
    }
}

impl Deref for Tokens {
    type Target = Palette;

    fn deref(&self) -> &Palette {
        &self.palette
    }
}

#[cfg(test)]
impl Tokens {
    /// Classic dark tokens: the baseline the GUI tests assert against.
    pub const DARK: Self = Self {
        palette: Palette::DARK,
        metrics: Metrics::CLASSIC,
    };

    /// Classic light tokens, for contrast assertions.
    pub const LIGHT: Self = Self {
        palette: Palette::LIGHT,
        metrics: Metrics::CLASSIC,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::theme::{CONTROL_H, FOOTER_H, HEADER_H};

    #[test]
    fn palette_follows_the_configured_theme() {
        assert_eq!(Palette::of(AppTheme::Dark), Palette::DARK);
        assert_eq!(Palette::of(AppTheme::Light), Palette::LIGHT);
        assert_eq!(
            Tokens::of(AppTheme::Dark, UiSkin::Classic),
            Tokens::DARK
        );
        assert_eq!(
            Tokens::of(AppTheme::Light, UiSkin::Classic),
            Tokens::LIGHT
        );
        assert_ne!(Tokens::DARK.palette, Tokens::LIGHT.palette);
    }

    #[test]
    fn the_two_skins_differ_in_geometry_but_share_the_palette() {
        let classic = Tokens::of(AppTheme::Dark, UiSkin::Classic);
        let modern = Tokens::of(AppTheme::Dark, UiSkin::Modern);
        assert_eq!(classic.palette(), modern.palette());
        assert_ne!(classic.metrics(), modern.metrics());
        assert_eq!(classic.metrics().card_radius, 0);
        assert!(modern.metrics().card_radius > classic.metrics().card_radius);
        assert!(modern.metrics().control_h > classic.metrics().control_h);
    }

    #[test]
    fn classic_metrics_keep_the_studio_geometry() {
        let t = Tokens::DARK;
        let metrics = t.metrics();
        assert_eq!(metrics.control_h, CONTROL_H);
        assert_eq!(metrics.header_h, HEADER_H);
        assert_eq!(metrics.footer_h, FOOTER_H);
        assert_eq!(metrics.card_radius, 0, "the classic skin is square");
        assert_eq!(metrics.control_radius, 0);
        assert_eq!(metrics.field_radius, 6);
        assert_eq!(metrics.surface_alpha, 255, "classic surfaces are opaque");
        assert_eq!(t.hairline(), Stroke::new(1.0, t.line));
    }

    #[test]
    fn modern_surfaces_are_translucent_and_lifted() {
        let modern = Tokens::of(AppTheme::Dark, UiSkin::Modern);
        assert!(modern.surface().a() < 255);
        assert!(modern.raised().r() > modern.surface().r());
        let classic = Tokens::of(AppTheme::Dark, UiSkin::Classic);
        assert_eq!(classic.surface(), classic.bg);
    }

    #[test]
    fn an_accent_override_re_tints_the_accent_roles() {
        assert_eq!(parse_accent(Some("#2d81ff")), Some([0x2d, 0x81, 0xff]));
        assert_eq!(parse_accent(Some("#2D81FF")), Some([0x2d, 0x81, 0xff]));
        assert_eq!(parse_accent(Some("not a colour")), None);
        assert_eq!(parse_accent(None), None);

        let tinted = Tokens::DARK.with_accent(Some([0x10, 0xb9, 0x81]));
        assert_eq!(tinted.accent, Color32::from_rgb(0x10, 0xb9, 0x81));
        assert_ne!(tinted.accent_ink, tinted.accent, "accent text is derived, not copied");
        assert_ne!(
            tinted.select_bg, Tokens::DARK.select_bg,
            "the selection background follows the accent"
        );
    }

    #[test]
    fn a_palette_cross_fade_blends_and_never_starts_without_time() {
        let start = Palette::lerp(Palette::DARK, Palette::LIGHT, 0.0);
        let end = Palette::lerp(Palette::DARK, Palette::LIGHT, 1.0);
        assert_eq!(start.bg, Palette::DARK.bg, "at zero the colours are still the old ones");
        assert_eq!(end.bg, Palette::LIGHT.bg, "at one the colours are the new ones");

        let halfway = Palette::lerp(Palette::DARK, Palette::LIGHT, 0.5);
        assert_ne!(halfway.bg, Palette::DARK.bg);
        assert_ne!(halfway.bg, Palette::LIGHT.bg);
        assert!(!halfway.dark, "the fade already reports the target's polarity");

        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        begin_palette_fade(&ctx, Palette::DARK, Palette::LIGHT, 0.0);
        assert_eq!(
            fading_palette(&ctx, Palette::LIGHT),
            Palette::LIGHT,
            "a zero duration never starts a fade"
        );
        assert_eq!(
            palette_fade_status(&ctx, Palette::LIGHT),
            FadeStatus::Idle,
            "no fade means no state to settle"
        );
    }
}