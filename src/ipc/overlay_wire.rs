//! Overlay-definition wire encoding shared by the daemon overlay manager (writer) and the
//! standalone `--overlay-window` process (reader).

use crate::models::OverlayDefinition;

pub fn decode_overlay_b64(b64: &str) -> Option<OverlayDefinition> {
    use base64::Engine;
    use std::io::Read;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let mut decoder = flate2::read::DeflateDecoder::new(&bytes[..]);
    let mut json_str = String::new();
    decoder.read_to_string(&mut json_str).ok()?;
    serde_json::from_str(&json_str).ok()
}

pub fn encode_overlay_b64(def: &OverlayDefinition) -> Option<String> {
    use base64::Engine;
    use std::io::Write;
    let json = serde_json::to_string(def).ok()?;
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(json.as_bytes()).ok()?;
    let compressed = encoder.finish().ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(compressed))
}

#[cfg(test)]
mod tests {
    use super::{decode_overlay_b64, encode_overlay_b64};
    use crate::models::OverlayDefinition;

    #[test]
    fn test_overlay_window_encode_decode_b64() {
        let mut def = OverlayDefinition::default_new_overlay();
        def.id = "b64_test".to_string();
        def.name = "Test B64 Overlay".to_string();
        def.fade_in_ms = Some(200);
        def.fade_out_ms = Some(350);

        let encoded = encode_overlay_b64(&def).expect("Encoding failed");
        let decoded = decode_overlay_b64(&encoded).expect("Decoding failed");

        assert_eq!(decoded.id, "b64_test");
        assert_eq!(decoded.name, "Test B64 Overlay");
        assert_eq!(decoded.fade_in_ms, Some(200));
        assert_eq!(decoded.fade_out_ms, Some(350));
    }
}
