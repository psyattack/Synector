use super::catalog::{catalog_placeholders, collect_rust_sources, references_field};
use super::{get, Language};
use std::collections::BTreeSet;
use std::path::Path;

#[test]
fn test_language_defaults_and_labels() {
    assert_eq!(Language::default(), Language::English);
    assert_eq!(Language::English.label(), "ENGLISH");
    assert_eq!(Language::Russian.label(), "РУССКИЙ");
}

#[test]
fn test_language_serde_roundtrip() {
    let json_en = serde_json::to_string(&Language::English).unwrap();
    assert_eq!(json_en, "\"english\"");
    let parsed_en: Language = serde_json::from_str(&json_en).unwrap();
    assert_eq!(parsed_en, Language::English);

    let json_ru = serde_json::to_string(&Language::Russian).unwrap();
    assert_eq!(json_ru, "\"russian\"");
    let parsed_ru: Language = serde_json::from_str(&json_ru).unwrap();
    assert_eq!(parsed_ru, Language::Russian);
}

#[test]
fn test_get_language_catalogs() {
    let en = get(Language::English);
    let ru = get(Language::Russian);

    assert_eq!(en.header_analytics_tip, "Analytics dashboard");
    assert_eq!(ru.header_analytics_tip, "Панель аналитики");
    assert_eq!(en.header_settings_tip, "Settings");
    assert_eq!(ru.header_settings_tip, "Настройки");

    assert_eq!(en.rules_tag_filter_all, "All Tags");
    assert_eq!(ru.rules_tag_filter_all, "Все теги");

    assert_eq!(en.settings_theme_dark, "Dark");
    assert_eq!(ru.settings_theme_dark, "Тёмная");

    assert_eq!(en.footer_btn_update, "UPDATE");
    assert_eq!(ru.footer_btn_update, "ОБНОВИТЬ");
    assert_eq!(en.footer_btn_info, "INFO");
    assert_eq!(ru.footer_btn_info, "ИНФО");
}

/// The catalogs must agree on every field and on the placeholder spelling
/// inside each value, so a value can be formatted from either language.
#[test]
fn test_catalog_placeholder_parity() {
    let en = catalog_placeholders(include_str!("en.rs"));
    let ru = catalog_placeholders(include_str!("ru.rs"));

    assert!(!en.is_empty(), "no fields parsed from en.rs");
    assert!(
        en.keys().eq(ru.keys()),
        "en.rs and ru.rs declare different field sets"
    );

    for (field, en_placeholders) in &en {
        let ru_placeholders = &ru[*field];
        assert_eq!(
            en_placeholders, ru_placeholders,
            "placeholder mismatch in catalog field `{field}`"
        );
    }
}

#[test]
fn test_catalog_parser_handles_escaped_quotes() {
    let parsed = catalog_placeholders(
        "field_one: \"say \\\"{token}\\\" here\",\nfield_two: \"{}\",\n// ignored: \"x\"\n",
    );

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed["field_one"], BTreeSet::from(["{token}"]));
    assert_eq!(parsed["field_two"], BTreeSet::from(["{}"]));
}

/// Every `Strings` field must be consumed by real code outside the catalog:
/// a field only the catalog mentions is dead UI surface (U16-001). The
/// source tree is present in a checkout but not necessarily in a packaged
/// build, so an unreadable `src/` skips the check instead of failing it.
#[test]
fn test_every_strings_field_is_referenced() {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = Vec::new();
    if collect_rust_sources(&src_root, &mut sources).is_err() {
        return;
    }

    // Skip the field declaration, both catalogs and the test machinery that
    // lives beside them: `src/i18n.rs` plus every `.rs` file directly under
    // `src/i18n/`. Only production references may justify a field.
    let declaration_file = src_root.join("i18n.rs");
    let i18n_dir = src_root.join("i18n");
    let mut code = String::new();
    for source in &sources {
        if source == &declaration_file || source.parent() == Some(i18n_dir.as_path()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(source) else {
            continue;
        };
        for line in text.lines() {
            code.push_str(line.split("//").next().unwrap_or(""));
            code.push('\n');
        }
    }

    let declaration = include_str!("strings.rs");
    let mut fields = Vec::new();
    for line in declaration.lines() {
        let Some(rest) = line.trim_start().strip_prefix("pub ") else {
            continue;
        };
        let Some((name, _)) = rest.split_once(':') else {
            continue;
        };
        if rest.trim_end().ends_with("&'static str,") {
            fields.push(name);
        }
    }
    assert!(
        !fields.is_empty(),
        "no `Strings` fields parsed from strings.rs"
    );

    let unused: Vec<_> = fields
        .into_iter()
        .filter(|field| !references_field(&code, field))
        .collect();
    assert!(
        unused.is_empty(),
        "`Strings` fields referenced nowhere outside the i18n catalog: {unused:?}"
    );
}
