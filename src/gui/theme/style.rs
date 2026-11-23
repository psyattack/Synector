//! Derives egui's `Visuals`, spacing and text styles from the resolved tokens.
//!
//! Everything a native egui widget renders comes from here, so the token layer is also the one
//! place where the framed-control height, the widget corner radius, the popup radius, the
//! palette and the motion permissions are decided.

use egui::{Color32, CornerRadius, Stroke, Visuals};

use crate::config::{AppTheme, UiSkin};

use super::fonts::medium;
use super::motion;
use super::tokens::{Palette, Tokens};

/// Natural outer height of a framed control (DragValue, ComboBox, Button): the button
/// text plus the theme's button padding on both sides. Keeping `interact_size.y` at this
/// value makes all of them take exactly the same height instead of a mix of natural and
/// minimum sizes, which is what made a ComboBox sit lower than a DragValue next to it.
pub fn framed_control_height(ui: &egui::Ui) -> f32 {
    let text_h = ui.text_style_height(&egui::TextStyle::Button);
    (text_h + 2.0 * ui.spacing().button_padding.y).ceil()
}

/// The control height of the active skin. `apply_theme` writes it into egui's spacing, so a view
/// asks for it here instead of repeating a literal, and a dense local layout can still lower it
/// for its own scope.
pub fn control_height(ui: &egui::Ui) -> f32 {
    ui.spacing().interact_size.y
}

pub fn shadow(t: &Tokens) -> egui::epaint::Shadow {
    let spec = t.metrics().shadow;
    egui::epaint::Shadow {
        offset: [0, spec.offset_y],
        blur: spec.blur,
        spread: 0,
        color: Color32::from_black_alpha(if t.dark {
            spec.dark_alpha
        } else {
            spec.light_alpha
        }),
    }
}

pub fn scrim(t: &Tokens, amount: f32) -> Color32 {
    let alpha = if t.dark { 170.0 } else { 105.0 } * amount.clamp(0.0, 1.0);
    Color32::from_black_alpha(alpha.round() as u8)
}

fn visuals(t: &Tokens) -> Visuals {
    let m = t.metrics();
    let modern = m.skin == UiSkin::Modern;
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.bg;
    v.window_fill = t.bg;
    v.extreme_bg_color = t.sunken;
    v.text_edit_bg_color = Some(if modern { t.glass_button(0.0) } else { t.sunken });
    v.faint_bg_color = t.sunken;
    v.code_bg_color = t.sunken;
    v.window_stroke = Stroke::new(m.hairline_w, t.line_strong);
    v.window_corner_radius = CornerRadius::same(m.popup_radius);
    v.menu_corner_radius = CornerRadius::same(m.popup_radius);
    v.window_shadow = shadow(t);
    v.popup_shadow = shadow(t);
    v.override_text_color = Some(t.ink);
    v.hyperlink_color = t.accent_ink;
    v.selection.bg_fill = t.select_bg;
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.warn_fg_color = t.warn;
    v.error_fg_color = t.bad;

    // Native widgets (ComboBox, DragValue, checkboxes, default-framed text edits) share the
    // field material: a translucent tint with the same border as the secondary buttons. The
    // classic generation keeps its flat, opaque chrome.
    let rest_fill = if modern { t.glass_button(0.0) } else { t.bg };
    let hover_fill = if modern { t.glass_button(0.6) } else { t.raised() };
    let active_fill = if modern { t.glass_button(1.0) } else { t.raised() };
    let rest_stroke = if modern {
        t.line.lerp_to_gamma(t.line_strong, 0.55)
    } else {
        t.line
    };
    for (w, fill, stroke, fg) in [
        (&mut v.widgets.noninteractive, t.bg, t.line, t.muted),
        (&mut v.widgets.inactive, rest_fill, rest_stroke, t.ink),
        (&mut v.widgets.hovered, hover_fill, t.line_strong, t.ink_strong),
        (&mut v.widgets.active, active_fill, t.line_strong, t.ink_strong),
        (&mut v.widgets.open, active_fill, t.line_strong, t.ink_strong),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(m.hairline_w, stroke);
        w.corner_radius = CornerRadius::same(m.control_radius);
        w.fg_stroke = Stroke::new(1.0, fg);
        w.expansion = 0.0;
    }
    v
}

pub fn apply_theme(ctx: &egui::Context, theme: AppTheme, skin: UiSkin, accent: Option<[u8; 3]>) {
    let previous_theme = ctx.data(|data| data.get_temp::<AppTheme>(egui::Id::unique("app_theme")));
    let previous_accent = ctx
        .data(|data| data.get_temp::<Option<[u8; 3]>>(super::tokens::accent_id()))
        .flatten();
    let fade_from =
        previous_theme.map(|theme| Palette::of(theme).with_optional_accent(previous_accent));
    let tokens = Tokens::of(theme, skin).with_accent(accent);
    ctx.data_mut(|d| {
        d.insert_temp(egui::Id::unique("app_theme"), theme);
        d.insert_temp(egui::Id::unique("app_skin"), skin);
        d.insert_temp(super::tokens::accent_id(), accent);
    });

    // A palette change cross-fades; a skin change never does, because its palette is unchanged.
    // An accent change is applied at once: fading it would chase the colour picker.
    if let Some(from) = fade_from {
        if from != tokens.palette() {
            super::tokens::begin_palette_fade(ctx, from, tokens.palette(), motion::duration(0.18));
        }
    }

    // The preference must be set before the visuals: `Context::set_visuals` writes into
    // whichever egui theme is currently active, so a palette applied while the other egui
    // theme is active would land in the wrong style while `Tokens::get` already reported the
    // new one — native widgets then rendered with the plain default palette.
    ctx.set_theme(if tokens.dark {
        egui::ThemePreference::Dark
    } else {
        egui::ThemePreference::Light
    });
    ctx.set_visuals_of(egui::Theme::Dark, visuals(&Tokens::of(AppTheme::Dark, skin)));
    ctx.set_visuals_of(egui::Theme::Light, visuals(&Tokens::of(AppTheme::Light, skin)));
    ctx.set_visuals(visuals(&tokens));

    let metrics = tokens.metrics();
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.icon_spacing = 8.0;
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.interact_size.y = metrics.control_h;
        style.spacing.menu_margin = egui::Margin::same(4);
        style.spacing.window_margin = egui::Margin::same(4);
        // Popups, tooltips and smooth scrolling inherit this; a skin with motion disabled
        // collapses it to zero so no transition schedules a repaint.
        style.animation_time = motion::duration(metrics.motion.base);
        style.text_styles = [
            (
                egui::TextStyle::Heading,
                egui::FontId::new(22.0, super::fonts::display()),
            ),
            (egui::TextStyle::Body, egui::FontId::new(13.5, egui::FontFamily::Proportional)),
            (egui::TextStyle::Button, egui::FontId::new(12.5, medium())),
            (egui::TextStyle::Small, egui::FontId::new(11.5, egui::FontFamily::Proportional)),
            (egui::TextStyle::Monospace, egui::FontId::new(12.5, egui::FontFamily::Monospace)),
        ]
        .into();
    });
}

/// Re-derives the native widget visuals while a palette cross-fade runs, applies the final
/// visuals on the frame the fade ends, and keeps requesting frames until it settles. Without
/// that last step the style layer would stay at the last blended colour until the next
/// `apply_theme`, because nothing else writes the visuals of the active egui theme.
pub fn tick_palette_fade(ctx: &egui::Context) {
    let theme = ctx
        .data(|data| data.get_temp::<AppTheme>(egui::Id::unique("app_theme")))
        .unwrap_or(AppTheme::Dark);
    let accent = ctx
        .data(|data| data.get_temp::<Option<[u8; 3]>>(super::tokens::accent_id()))
        .flatten();
    let target = Palette::of(theme).with_optional_accent(accent);
    let status = super::tokens::palette_fade_status(ctx, target);
    if status == super::tokens::FadeStatus::Idle {
        return;
    }
    ctx.set_visuals(visuals(&Tokens::get(ctx)));
    if status == super::tokens::FadeStatus::Running {
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_theme_applies_the_palette_to_the_active_egui_theme() {
        let ctx = egui::Context::default();

        // The regression scenario: apply a dark palette while the Light preference is active.
        // The palette must land in the Dark style, not stay in Light, while `Tokens::get`
        // reports the new theme from the first frame on.
        apply_theme(&ctx, AppTheme::Light, UiSkin::Classic, None);
        super::super::tokens::finish_palette_fade(&ctx);
        assert_eq!(Tokens::get(&ctx), Tokens::LIGHT);
        assert_eq!(ctx.global_style().visuals.panel_fill, Tokens::LIGHT.bg);

        apply_theme(&ctx, AppTheme::Dark, UiSkin::Classic, None);
        // A theme change starts a cross-fade; settle it before asserting the target palette.
        super::super::tokens::finish_palette_fade(&ctx);
        assert_eq!(Tokens::get(&ctx), Tokens::DARK);
        assert_eq!(ctx.global_style().visuals.panel_fill, Tokens::DARK.bg);
        assert_eq!(
            ctx.global_style().visuals.text_edit_bg_color,
            Some(Tokens::DARK.sunken)
        );
    }

    #[test]
    fn a_finished_palette_fade_lands_the_target_visuals() {
        let ctx = egui::Context::default();
        apply_theme(&ctx, AppTheme::Light, UiSkin::Modern, None);
        // The fade is started explicitly because `apply_theme` sizes it from the platform motion
        // permission, which this test must not depend on.
        super::super::tokens::begin_palette_fade(&ctx, Palette::DARK, Palette::LIGHT, 0.18);

        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(0.0),
                ..Default::default()
            },
            |ui| tick_palette_fade(ui.ctx()),
        );
        output.textures_delta.clear();
        assert_ne!(
            ctx.global_style().visuals.panel_fill,
            Palette::LIGHT.bg,
            "the frame the fade starts must still paint the blend"
        );

        // Past the fade duration the target visuals are applied, not the last blend.
        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(1.0),
                ..Default::default()
            },
            |ui| tick_palette_fade(ui.ctx()),
        );
        output.textures_delta.clear();
        assert_eq!(ctx.global_style().visuals.panel_fill, Palette::LIGHT.bg);

        // Settling is one-shot: a later frame leaves the same target in place.
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| tick_palette_fade(ui.ctx()));
        output.textures_delta.clear();
        assert_eq!(ctx.global_style().visuals.panel_fill, Palette::LIGHT.bg);
    }

    #[test]
    fn modern_native_widgets_share_the_field_material() {
        let ctx = egui::Context::default();
        apply_theme(&ctx, AppTheme::Dark, UiSkin::Modern, None);
        let tokens = Tokens::get(&ctx);
        let rest = tokens.glass_button(0.0);
        assert_eq!(ctx.global_style().visuals.widgets.inactive.bg_fill, rest);
        assert_eq!(ctx.global_style().visuals.text_edit_bg_color, Some(rest));
        assert!(rest.a() < 255, "native widgets are translucent in the modern skin");

        apply_theme(&ctx, AppTheme::Dark, UiSkin::Classic, None);
        assert_eq!(
            ctx.global_style().visuals.widgets.inactive.bg_fill,
            Tokens::of(AppTheme::Dark, UiSkin::Classic).bg,
            "the classic skin keeps its flat opaque chrome"
        );
    }

    #[test]
    fn applying_a_skin_keeps_the_palette_and_changes_the_geometry() {
        let ctx = egui::Context::default();
        apply_theme(&ctx, AppTheme::Dark, UiSkin::Modern, None);
        let modern = Tokens::get(&ctx);
        assert_eq!(modern.palette(), Tokens::DARK.palette());
        assert_eq!(modern.metrics().skin, UiSkin::Modern);
        assert_eq!(
            ctx.global_style().visuals.widgets.inactive.corner_radius,
            CornerRadius::same(modern.metrics().control_radius)
        );

        apply_theme(&ctx, AppTheme::Dark, UiSkin::Classic, None);
        assert_eq!(Tokens::get(&ctx).metrics().skin, UiSkin::Classic);
        assert_eq!(
            ctx.global_style().visuals.widgets.inactive.corner_radius,
            CornerRadius::same(0)
        );
    }
}
