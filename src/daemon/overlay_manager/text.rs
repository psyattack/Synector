use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

static OVERLAY_TEXTS: OnceLock<Arc<Mutex<HashMap<String, String>>>> = OnceLock::new();

pub(super) fn texts_map() -> &'static Arc<Mutex<HashMap<String, String>>> {
    OVERLAY_TEXTS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

pub fn get_overlay_text(id: &str) -> Option<String> {
    texts_map().lock().unwrap().get(id).cloned()
}

fn live_text_dir() -> std::path::PathBuf {
    crate::config::AppConfig::config_dir().join("overlay_text")
}

/// Path of the live-text file for `id`. Overlay windows poll this file so a
/// `SetText` update repaints the label instead of restarting the process.
pub fn live_overlay_text_path(id: &str) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let slug: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(48)
        .collect();
    let digest = Sha256::digest(id.as_bytes());
    let suffix = digest[..4].iter().map(|b| format!("{b:02x}")).collect::<String>();
    live_text_dir().join(format!("{slug}-{suffix}.txt"))
}

pub(super) fn write_live_overlay_text(id: &str, text: &str) {
    static WARN_ONCE: std::sync::Once = std::sync::Once::new();
    let path = live_overlay_text_path(id);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    let tmp = std::path::PathBuf::from(tmp_name);
    let written = std::fs::write(&tmp, text).is_ok()
        && match std::fs::rename(&tmp, &path) {
            Ok(()) => true,
            Err(_) => {
                let _ = std::fs::remove_file(&tmp);
                false
            }
        };
    if !written {
        WARN_ONCE.call_once(|| {
            crate::logging::warn(
                "OVERLAY",
                "Cannot publish live overlay text; SetText updates will not be shown",
            );
        });
    }
}

/// Latest live text published for `id`, read by the overlay window. `None`
/// means no text has been published for this overlay yet.
pub fn live_overlay_text(id: &str) -> Option<String> {
    std::fs::read_to_string(live_overlay_text_path(id)).ok()
}

pub fn update_overlay_text(id_or_name: &str, text: &str) {
    let config = crate::config::Library::shared();
    let resolved_id = config
        .find_overlay(id_or_name)
        .map(|o| o.id.clone())
        .unwrap_or_else(|| id_or_name.to_string());
    texts_map().lock().unwrap().insert(resolved_id.clone(), text.to_string());
    // Publish the new label for the running window; it polls this file, so no
    // kill/respawn (and no flicker or GPU startup storm) per text frame.
    write_live_overlay_text(&resolved_id, text);
}

/// Returns the live text only when it differs from `current`, so the overlay
/// window can swap it without re-reading every frame.
pub fn live_overlay_text_if_changed(id: &str, current: Option<&str>) -> Option<String> {
    let text = live_overlay_text(id)?;
    if current == Some(text.as_str()) {
        None
    } else {
        Some(text)
    }
}
