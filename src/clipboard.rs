//! System clipboard access shared by the GUI and daemon action handlers.
//!
//! Kept outside `sharing` so share codes and token expansion do not depend on the codec tree.

/// Copy text to system clipboard via both egui context and arboard for maximum reliability
pub fn copy_text_to_clipboard(ctx: &egui::Context, text: &str) {
    ctx.copy_text(text.to_string());
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text);
    }
}

/// Read text from system clipboard
pub fn read_text() -> Result<String, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("Clipboard error: {e}"))?;
    cb.get_text().map_err(|e| format!("Clipboard empty or non-text: {e}"))
}
