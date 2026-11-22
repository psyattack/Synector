use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Parse `field: "value",` catalog lines into field name → `{...}`
/// placeholder set. Escaped quotes do not terminate the value, so the
/// parser keeps working when a future value contains `\"`.
pub(super) fn catalog_placeholders(source: &str) -> BTreeMap<&str, BTreeSet<&str>> {
    let mut fields = BTreeMap::new();
    for line in source.lines() {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let Some(colon) = line.find(':') else {
            continue;
        };
        let name = line[..colon].trim();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            continue;
        }
        let Some(open) = line[colon + 1..].find('"') else {
            continue;
        };
        let Some(value) = quoted_value(&line[colon + 1 + open + 1..]) else {
            continue;
        };
        fields.insert(name, placeholders(value));
    }
    fields
}

/// Slice a Rust string literal body at its first unescaped `"`.
fn quoted_value(text: &str) -> Option<&str> {
    let mut chars = text.char_indices();
    loop {
        let (i, c) = chars.next()?;
        match c {
            '\\' => {
                chars.next()?;
            }
            '"' => return Some(&text[..i]),
            _ => {}
        }
    }
}

/// Collect the `{...}` tokens of a catalog value (deduplicated).
fn placeholders(value: &str) -> BTreeSet<&str> {
    let mut set = BTreeSet::new();
    let mut rest = value;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open + 1..].find('}') else {
            break;
        };
        set.insert(&rest[open..open + close + 2]);
        rest = &rest[open + close + 2..];
    }
    set
}

/// Recursively collect every `.rs` file under `dir`.
pub(super) fn collect_rust_sources(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_rust_sources(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// True when `haystack` contains `.<field>` as a complete identifier.
pub(super) fn references_field(haystack: &str, field: &str) -> bool {
    let needle = format!(".{field}");
    haystack.match_indices(&needle).any(|(index, _)| {
        haystack[index + needle.len()..]
            .chars()
            .next()
            .is_none_or(|next| !(next.is_ascii_alphanumeric() || next == '_'))
    })
}
