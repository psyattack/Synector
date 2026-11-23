//! Fonts: the bundled type stacks, the per-role families and their registration.
//!
//! The families are registered once per context ([`install_fonts`]); which of them a given text
//! style uses is decided by the style layer, so a skin can change the typographic voice without
//! reloading any font file.

use std::sync::Arc;

use egui::epaint::text::VariationCoords;
use egui::{FontData, FontDefinitions, FontFamily, FontTweak};

/// Archivo 620 display font for headings and branding.
pub fn display() -> FontFamily {
    FontFamily::Name("display".into())
}

/// Archivo Regular (400 weight).
pub fn font_archivo() -> FontFamily {
    FontFamily::Name("archivo".into())
}

/// Archivo Bold (700 weight).
pub fn font_archivo_bold() -> FontFamily {
    FontFamily::Name("archivo-bold".into())
}

/// Inter 500 font for medium body copy.
pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

/// Inter 600 font for semibold body copy.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Geist Mono 500 for precision labels and tags.
pub fn mono() -> FontFamily {
    FontFamily::Name("mono".into())
}

/// Geist Mono 700 for button titles and active counts.
pub fn mono_bold() -> FontFamily {
    FontFamily::Name("mono-bold".into())
}

/// Phosphor icon font family.
pub fn phosphor() -> FontFamily {
    FontFamily::Name("phosphor".into())
}



type Face<'a> = (&'a str, &'static [u8], &'a [([u8; 4], f32)], FontFamily);

pub fn install_fonts(ctx: &egui::Context) {
    ctx.set_fonts(build_fonts());
}

/// Builds the full font set. Kept separate from [`install_fonts`] so tests can
/// verify that icon and emoji fallbacks stay registered.
pub(crate) fn build_fonts() -> FontDefinitions {
    const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Variable.ttf");
    const ARCHIVO: &[u8] = include_bytes!("../../../assets/fonts/Archivo-Variable.ttf");
    const GEIST_MONO: &[u8] = include_bytes!("../../../assets/fonts/GeistMono-Variable.ttf");

    let mut fonts = FontDefinitions::default();

    // Register Phosphor Icons into the font system
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    fonts.families.insert(phosphor(), vec!["phosphor".into()]);

    let mut fallback: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fallback.retain(|f| f != "phosphor");
    fallback.insert(0, "phosphor".into());

    let mut mono_fallback: Vec<String> = fonts
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    mono_fallback.retain(|f| f != "phosphor");
    mono_fallback.insert(0, "phosphor".into());

    let faces: [Face<'_>; 9] = [
        ("Inter", INTER, &[(*b"wght", 400.0), (*b"opsz", 14.0)], FontFamily::Proportional),
        ("Inter-Medium", INTER, &[(*b"wght", 500.0), (*b"opsz", 14.0)], medium()),
        ("Inter-SemiBold", INTER, &[(*b"wght", 600.0), (*b"opsz", 14.0)], semibold()),
        ("Archivo-Regular", ARCHIVO, &[(*b"wght", 400.0), (*b"wdth", 100.0)], font_archivo()),
        ("Archivo-Bold", ARCHIVO, &[(*b"wght", 700.0), (*b"wdth", 105.0)], font_archivo_bold()),
        ("Archivo-Display", ARCHIVO, &[(*b"wght", 620.0), (*b"wdth", 118.0)], display()),
        ("GeistMono", GEIST_MONO, &[(*b"wght", 400.0)], FontFamily::Monospace),
        ("GeistMono-Medium", GEIST_MONO, &[(*b"wght", 500.0)], mono()),
        ("GeistMono-Bold", GEIST_MONO, &[(*b"wght", 700.0)], mono_bold()),
    ];

    for (name, bytes, coords, family) in faces {
        let tweak = FontTweak {
            coords: VariationCoords::new(coords.iter().copied()),
            ..Default::default()
        };
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes).tweak(tweak)));

        let mut stack = vec![name.to_owned()];
        if family == FontFamily::Monospace || family == mono() || family == mono_bold() {
            stack.extend(mono_fallback.iter().cloned());
        } else {
            stack.extend(fallback.iter().cloned());
        }
        if !stack.iter().any(|f| f == "phosphor") {
            stack.push("phosphor".into());
        }
        fonts.families.insert(family, stack);
    }

    #[cfg(windows)]
    {
        let sys_fonts: &[(&str, &str)] = &[
            ("arial", "C:\\Windows\\Fonts\\arial.ttf"),
            ("arial-bold", "C:\\Windows\\Fonts\\arialbd.ttf"),
            ("impact", "C:\\Windows\\Fonts\\impact.ttf"),
            ("segoe ui", "C:\\Windows\\Fonts\\segoeui.ttf"),
            ("segoe ui-bold", "C:\\Windows\\Fonts\\segoeuib.ttf"),
            ("consolas", "C:\\Windows\\Fonts\\consola.ttf"),
            ("consolas-bold", "C:\\Windows\\Fonts\\consolab.ttf"),
            ("tahoma", "C:\\Windows\\Fonts\\tahoma.ttf"),
            ("tahoma-bold", "C:\\Windows\\Fonts\\tahomabd.ttf"),
            ("verdana", "C:\\Windows\\Fonts\\verdana.ttf"),
            ("verdana-bold", "C:\\Windows\\Fonts\\verdanab.ttf"),
            ("comic sans ms", "C:\\Windows\\Fonts\\comic.ttf"),
            ("comic sans ms-bold", "C:\\Windows\\Fonts\\comicbd.ttf"),
            ("georgia", "C:\\Windows\\Fonts\\georgia.ttf"),
            ("georgia-bold", "C:\\Windows\\Fonts\\georgiab.ttf"),
            ("trebuchet ms", "C:\\Windows\\Fonts\\trebuc.ttf"),
            ("trebuchet ms-bold", "C:\\Windows\\Fonts\\trebucbd.ttf"),
            ("courier new", "C:\\Windows\\Fonts\\cour.ttf"),
            ("courier new-bold", "C:\\Windows\\Fonts\\courbd.ttf"),
        ];
        for (name, path) in sys_fonts {
            if let Ok(bytes) = std::fs::read(path) {
                let font_name = (*name).to_string();
                fonts
                    .font_data
                    .insert(font_name.clone(), Arc::new(FontData::from_owned(bytes)));
                let mut stack = vec![font_name.clone()];
                stack.extend(fallback.iter().cloned());
                fonts
                    .families
                    .insert(FontFamily::Name(font_name.into()), stack);
            }
        }
    }

    fonts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_register_phosphor_and_bundled_emoji_fallbacks() {
        let fonts = build_fonts();
        for name in ["phosphor", "NotoEmoji-Regular", "emoji-icon-font"] {
            assert!(
                fonts.font_data.contains_key(name),
                "{name} is missing from font_data"
            );
        }
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            let stack = fonts.families.get(&family).expect("font family");
            for name in ["phosphor", "NotoEmoji-Regular", "emoji-icon-font"] {
                assert!(
                    stack.iter().any(|font| font == name),
                    "{name} is missing from {family:?} fallback"
                );
            }
        }
    }
}
