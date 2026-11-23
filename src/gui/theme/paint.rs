//! Shared painting helpers: the window backdrop, and model values turned into paint results.
//!
//! Owner of the presentation helpers that are neither colour roles nor geometry metrics, so a
//! view does not grow its own copy of the resolution rules.

use egui::{Color32, Context, CornerRadius, Mesh, Shape};
use egui::epaint::Vertex;

use crate::config::UiSkin;

use super::tokens::Tokens;

/// Paints the surface every panel sits on.
///
/// The classic skin paints the flat page colour. The modern skin paints a quiet two-stop wash
/// instead, because its panels are translucent ([`Tokens::surface`]) and need something layered
/// to sit on; the wash is derived from the palette, never from a decorative accent, so it works
/// in both palettes and never turns into a glow.
pub(crate) fn paint_backdrop(ctx: &Context, t: &Tokens) {
    let rect = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());

    if t.metrics().skin == UiSkin::Classic {
        painter.rect_filled(rect, CornerRadius::ZERO, t.bg);
        return;
    }

    let (top, bottom) = if t.dark {
        (t.bg.gamma_multiply(1.12), t.bg.gamma_multiply(0.88))
    } else {
        (t.bg, t.bg.gamma_multiply(0.93))
    };

    let mut mesh = Mesh::default();
    let base = mesh.vertices.len() as u32;
    let uv = egui::epaint::WHITE_UV;
    mesh.vertices.push(Vertex {
        pos: rect.left_top(),
        color: top,
        uv,
    });
    mesh.vertices.push(Vertex {
        pos: rect.right_top(),
        color: top,
        uv,
    });
    mesh.vertices.push(Vertex {
        pos: rect.left_bottom(),
        color: bottom,
        uv,
    });
    mesh.vertices.push(Vertex {
        pos: rect.right_bottom(),
        color: bottom,
        uv,
    });
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 1, base + 3]);
    painter.add(Shape::mesh(mesh));
}

/// A recessed bordered surface: a well, a card body or an inset panel.
///
/// Views use this instead of repeating `Frame::new().fill(sunken).stroke(hairline)` with a
/// literal radius, so an inset surface follows the skin.
pub(crate) fn well(t: &Tokens) -> egui::Frame {
    egui::Frame::new()
        .fill(t.sunken)
        .stroke(t.hairline())
        .corner_radius(t.card_corner())
}

/// Resolves a rule tag colour (`#RRGGBB`, parsed by the model) against a palette fallback.
/// The model owns the parser; this is the GUI-side presentation of a tag colour.
pub(crate) fn tag_color(tag: &crate::models::RuleTag, fallback: Color32) -> Color32 {
    tag.color
        .as_deref()
        .and_then(crate::models::parse_hex_color)
        .map(|[r, g, b]| Color32::from_rgb(r, g, b))
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppTheme;
    use crate::models::RuleTag;

    #[test]
    fn backdrop_paints_one_layer_for_both_skins_and_palettes() {
        for skin in [UiSkin::Classic, UiSkin::Modern] {
            for theme in [AppTheme::Dark, AppTheme::Light] {
                let ctx = egui::Context::default();
                let tokens = Tokens::of(theme, skin);
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    paint_backdrop(ui.ctx(), &tokens);
                });
                // A test does not upload textures, and an unhandled delta panics on drop.
                output.textures_delta.clear();
                assert!(
                    !output.shapes.is_empty(),
                    "the backdrop must paint something for {skin:?}/{theme:?}"
                );
            }
        }
    }

    #[test]
    fn rule_tag_serde_and_color_parsing_round_trip() {
        let tag = RuleTag::new("Productivity")
            .with_color("#10B981")
            .with_icon("⚡");

        let json = serde_json::to_string(&tag).unwrap();
        let decoded: RuleTag = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.name, "Productivity");
        assert_eq!(decoded.color.as_deref(), Some("#10B981"));
        assert_eq!(decoded.icon.as_deref(), Some("⚡"));

        let color = tag_color(&decoded, Color32::WHITE);
        assert_eq!(color, Color32::from_rgb(0x10, 0xB9, 0x81));

        let invalid_color_tag = RuleTag::new("Bad").with_color("not_a_color");
        let fallback = Color32::from_rgb(1, 2, 3);
        assert_eq!(tag_color(&invalid_color_tag, fallback), fallback);

        // A non-ASCII colour is rejected by the model parser and keeps the fallback.
        let broken = RuleTag::new("Broken").with_color("a\u{e9}\u{20ac}");
        assert_eq!(tag_color(&broken, fallback), fallback);
    }
}
