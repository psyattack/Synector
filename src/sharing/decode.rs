//! Share code decoding: bounded decompression, checksum comparison and payload validation.

use std::io::Read;

use base64::prelude::*;
use sha2::{Digest, Sha256};

use crate::models::Rule;

use super::format::{
    ShareError, CHECKSUM_LEN, SHARE_DICTIONARY, SHARE_PREFIX, SHARE_VERSION, MAX_CODE_CHARS,
    MAX_PAYLOAD_BYTES, MAX_SHARE_OVERLAYS, MAX_SHARE_PRESETS, MAX_SHARE_RULES,
};
use super::payload::SharePayload;

/// Decompresses a payload encoded with [`SHARE_DICTIONARY`]; bounded by [`MAX_PAYLOAD_BYTES`].
fn brotli_decompress_with_dictionary(compressed: &[u8]) -> Result<Vec<u8>, ShareError> {
    type HeapAllocU8 = brotli::HeapAlloc<u8>;
    let dictionary: <HeapAllocU8 as brotli::Allocator<u8>>::AllocatedMemory =
        SHARE_DICTIONARY.to_vec().into();
    let mut decoder = brotli::Decompressor::new_with_custom_dict(compressed, 4096, dictionary);
    let mut json_bytes = Vec::new();
    decoder
        .by_ref()
        .take((MAX_PAYLOAD_BYTES + 1) as u64)
        .read_to_end(&mut json_bytes)
        .map_err(|_| ShareError::CorruptedPayload)?;
    if json_bytes.len() > MAX_PAYLOAD_BYTES {
        return Err(ShareError::PayloadTooLarge);
    }
    Ok(json_bytes)
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Sanitizes and decodes a pasted share code (or a raw JSON paste) into a payload.
///
/// Accepts the code wrapped in backticks/quotes and tolerates whitespace and line breaks
/// anywhere inside the encoded body, which is what chat clients insert when wrapping.
pub fn decode_and_decompress(raw_input: &str) -> Result<SharePayload, ShareError> {
    let clean = raw_input
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim();
    if clean.is_empty() {
        return Err(ShareError::EmptyInput);
    }
    if clean.len() > MAX_CODE_CHARS {
        return Err(ShareError::CodeTooLong);
    }

    if let Some(encoded) = clean.strip_prefix(SHARE_PREFIX) {
        return decode_code(encoded);
    }
    if clean.starts_with("PH1:") {
        return Err(ShareError::LegacyFormat);
    }
    // Plain JSON paste (for example a rule exported with --rules-export) stays supported.
    decode_raw_json(clean)
}

fn decode_code(encoded: &str) -> Result<SharePayload, ShareError> {
    let compressed = decode_base64(encoded)?;
    if compressed.len() <= CHECKSUM_LEN {
        return Err(ShareError::CorruptedPayload);
    }
    let (checksum, compressed_body) = compressed.split_at(CHECKSUM_LEN);

    let json_bytes = brotli_decompress_with_dictionary(compressed_body)?;

    let digest = Sha256::digest(&json_bytes);
    if !checksums_match(checksum, &digest) {
        return Err(ShareError::ChecksumMismatch);
    }

    let value: serde_json::Value =
        serde_json::from_slice(&json_bytes).map_err(|_| ShareError::InvalidPayload)?;
    validate_share_payload_value(&value)?;
    let payload: SharePayload =
        serde_json::from_value(value).map_err(|_| ShareError::InvalidPayload)?;
    validate_payload(payload)
}

/// Compares the truncated digest with an XOR accumulator instead of slice equality.
///
/// Slice equality stops at the first differing byte; the only integrity check in the format
/// must not reveal where a mismatch happened. The length check comes first because the digest
/// length is public.
fn checksums_match(checksum: &[u8], digest: &[u8]) -> bool {
    if checksum.len() != CHECKSUM_LEN || digest.len() < CHECKSUM_LEN {
        return false;
    }
    let mut difference = 0u8;
    for (expected, actual) in checksum.iter().zip(digest.iter()) {
        difference |= expected ^ actual;
    }
    difference == 0
}

fn decode_base64(encoded: &str) -> Result<Vec<u8>, ShareError> {
    let cleaned: String = encoded
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    if cleaned.is_empty() {
        return Err(ShareError::InvalidBase64);
    }
    // Tolerate both the URL-safe alphabet we emit and the standard one from older builds,
    // with or without trailing padding.
    let mut normalized = String::with_capacity(cleaned.len());
    for ch in cleaned.chars() {
        normalized.push(match ch {
            '-' => '+',
            '_' => '/',
            other => other,
        });
    }
    let unpadded = normalized.trim_end_matches('=');
    BASE64_STANDARD_NO_PAD
        .decode(unpadded)
        .map_err(|_| ShareError::InvalidBase64)
}

fn decode_raw_json(raw: &str) -> Result<SharePayload, ShareError> {
    if raw.len() > MAX_PAYLOAD_BYTES {
        return Err(ShareError::PayloadTooLarge);
    }
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|_| ShareError::InvalidPayload)?;
    let payload = match value {
        serde_json::Value::Object(object) if object.contains_key("rules") => {
            let value = serde_json::Value::Object(object);
            validate_share_payload_value(&value)?;
            serde_json::from_value::<SharePayload>(value).map_err(|_| ShareError::InvalidPayload)?
        }
        serde_json::Value::Object(_) => {
            validate_rule_value(&value)?;
            let rule: Rule = serde_json::from_value(value).map_err(|_| ShareError::InvalidPayload)?;
            SharePayload::rules_only(vec![rule])
        }
        serde_json::Value::Array(_) => {
            validate_rule_array(&value)?;
            let rules: Vec<Rule> =
                serde_json::from_value(value).map_err(|_| ShareError::InvalidPayload)?;
            SharePayload::rules_only(rules)
        }
        _ => return Err(ShareError::InvalidPayload),
    };
    validate_payload(payload)
}

/// Rejects unknown keys in a raw share payload before serde can drop them silently.
fn validate_share_payload_value(value: &serde_json::Value) -> Result<(), ShareError> {
    let Some(object) = value.as_object() else {
        return Err(ShareError::InvalidPayload);
    };
    const PAYLOAD_KEYS: &[&str] = &[
        "version",
        "app_version",
        "created_at",
        "rules",
        "overlays",
        "macro_presets",
    ];
    if object
        .keys()
        .any(|key| !PAYLOAD_KEYS.contains(&key.as_str()))
    {
        return Err(ShareError::InvalidPayload);
    }
    if let Some(rules) = object.get("rules") {
        validate_rule_array(rules)?;
    }
    Ok(())
}

fn validate_rule_array(value: &serde_json::Value) -> Result<(), ShareError> {
    let Some(rules) = value.as_array() else {
        return Ok(());
    };
    for rule in rules {
        validate_rule_value(rule)?;
    }
    Ok(())
}

fn validate_rule_value(value: &serde_json::Value) -> Result<(), ShareError> {
    if crate::models::validate_rule_keys(value).is_err() {
        return Err(ShareError::InvalidPayload);
    }
    // Parse once here so the expensive typed checks run before the caller parses the
    // payload; unknown keys are already gone, so the typed value is exact.
    let rule: Rule = serde_json::from_value(value.clone()).map_err(|_| ShareError::InvalidPayload)?;
    validate_rule_definition(&rule)
}

/// Value checks applied to every imported rule: tag colours, quota window bounds, schedule
/// fields, condition keys and wait durations.
fn validate_rule_definition(rule: &Rule) -> Result<(), ShareError> {
    let invalid = rule.tag_color_error().is_some()
        || rule
            .quota_period
            .and_then(crate::models::QuotaPeriod::validation_error)
            .is_some()
        || rule
            .trigger
            .as_ref()
            .is_some_and(|trigger| !trigger.schedule_errors().is_empty())
        || rule
            .conditions
            .iter()
            .any(|item| !item.condition.validation_errors().is_empty())
        || {
            let mut invalid = false;
            rule.visit_actions(&mut |action| {
                invalid |= !action.validation_errors().is_empty();
            });
            invalid
        };
    if invalid {
        Err(ShareError::InvalidPayload)
    } else {
        Ok(())
    }
}

pub(super) fn validate_payload(payload: SharePayload) -> Result<SharePayload, ShareError> {
    if payload.version > SHARE_VERSION {
        return Err(ShareError::UnsupportedVersion(payload.version));
    }
    validate_payload_limits(&payload)?;
    if payload.rules.is_empty() && payload.overlays.is_empty() && payload.macro_presets.is_empty() {
        return Err(ShareError::EmptyPayload);
    }
    Ok(payload)
}

/// Count bounds shared by encoding and importing.
pub(super) fn validate_payload_limits(payload: &SharePayload) -> Result<(), ShareError> {
    if payload.rules.len() > MAX_SHARE_RULES {
        return Err(ShareError::TooManyRules);
    }
    if payload.overlays.len() > MAX_SHARE_OVERLAYS
        || payload.macro_presets.len() > MAX_SHARE_PRESETS
    {
        return Err(ShareError::TooManyLibraryItems);
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    use brotli::enc::BrotliEncoderParams;

    use crate::sharing::build_payload;
    use crate::sharing::payload::fixtures::{encode, sample_rule};

    #[test]
    fn test_share_code_survives_corruption_checks() {
        let (payload, _) = build_payload(vec![sample_rule("checksum_rule")], &[], &[]);
        let encoded = encode(&payload);
        let body = encoded.strip_prefix(SHARE_PREFIX).unwrap();
        let mut bytes = BASE64_URL_SAFE_NO_PAD.decode(body).unwrap();
        // Flip a bit in the middle of the compressed body (after the checksum prefix).
        let middle = CHECKSUM_LEN + (bytes.len() - CHECKSUM_LEN) / 2;
        bytes[middle] ^= 0x40;
        let corrupted = format!(
            "{SHARE_PREFIX}{}",
            BASE64_URL_SAFE_NO_PAD.encode(bytes)
        );
        let error = decode_and_decompress(&corrupted).expect_err("corruption must be caught");
        assert!(
            matches!(
                error,
                ShareError::ChecksumMismatch
                    | ShareError::CorruptedPayload
                    | ShareError::InvalidPayload
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn test_legacy_ph1_code_is_rejected() {
        let (payload, _) = build_payload(vec![sample_rule("legacy_rule")], &[], &[]);
        let json = serde_json::to_vec(&payload).unwrap();
        let legacy = format!("PH1:{}", BASE64_STANDARD.encode(&json));
        assert_eq!(
            decode_and_decompress(&legacy).unwrap_err(),
            ShareError::LegacyFormat
        );
    }

    #[test]
    fn test_share_code_tolerates_wrapping_and_line_breaks() {
        let (payload, _) = build_payload(vec![sample_rule("wrapped_rule")], &[], &[]);
        let encoded = encode(&payload);

        let decoded = decode_and_decompress(&format!("```\n{encoded}\n```\n")).expect("fenced");
        assert_eq!(decoded.rules[0].id, "wrapped_rule");

        // Split the body across lines the way a chat client might wrap it.
        let body = encoded.strip_prefix(SHARE_PREFIX).unwrap();
        let broken: String = body
            .as_bytes()
            .chunks(37)
            .map(|chunk| std::str::from_utf8(chunk).unwrap().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let decoded = decode_and_decompress(&format!("{SHARE_PREFIX}\n{broken}")).expect("wrapped");
        assert_eq!(decoded.rules[0].id, "wrapped_rule");
    }

    #[test]
    fn test_whitespace_and_quote_tolerance() {
        let (payload, _) = build_payload(vec![sample_rule("quoted_rule")], &[], &[]);
        let encoded = encode(&payload);
        let decoded = decode_and_decompress(&format!(" \"{encoded}\" ")).expect("quoted");
        assert_eq!(decoded.rules[0].id, "quoted_rule");
    }

    #[test]
    fn test_raw_json_paste_still_imports() {
        let rule = sample_rule("json_rule");
        let json = serde_json::to_string(&rule).unwrap();
        let decoded = decode_and_decompress(&json).expect("raw rule json");
        assert_eq!(decoded.rules.len(), 1);
        assert_eq!(decoded.rules[0].id, "json_rule");

        let batch_json = serde_json::to_string(&vec![sample_rule("a"), sample_rule("b")]).unwrap();
        let decoded = decode_and_decompress(&batch_json).expect("raw batch json");
        assert_eq!(decoded.rules.len(), 2);
    }

    #[test]
    fn test_import_rejects_unknown_keys_and_invalid_values() {
        let typo = r#"{"id":"r","name":"R","max_execution":5,"actions":[]}"#;
        assert_eq!(decode_and_decompress(typo), Err(ShareError::InvalidPayload));

        let nested = r#"{"id":"r","name":"R","actions":[{"$action_type":"notification","title":"T","message":"M","mesage":"typo"}]}"#;
        assert_eq!(decode_and_decompress(nested), Err(ShareError::InvalidPayload));

        // A tag colour that is not ASCII hex must not reach the renderer.
        let bad_tag = r#"{"id":"r","name":"R","tags":[{"name":"T","color":"aé€"}],"actions":[]}"#;
        assert_eq!(decode_and_decompress(bad_tag), Err(ShareError::InvalidPayload));

        // A Daily schedule without its time used to import and then never fire.
        let bad_schedule = r#"{"id":"r","name":"R","trigger":{"$trigger_type":"schedule","schedule_type":"daily","interval_sec":0},"actions":[]}"#;
        assert_eq!(
            decode_and_decompress(bad_schedule),
            Err(ShareError::InvalidPayload)
        );
    }

    #[test]
    fn test_checksum_bounds_the_decompressed_payload() {
        let oversized = vec![b' '; MAX_PAYLOAD_BYTES + 4096];
        let checksum = Sha256::digest(&oversized);
        let mut compressed = Vec::new();
        let params = BrotliEncoderParams {
            quality: 1,
            ..Default::default()
        };
        brotli::BrotliCompress(&mut oversized.as_slice(), &mut compressed, &params).unwrap();
        let mut framed = Vec::new();
        framed.extend_from_slice(&checksum[..CHECKSUM_LEN]);
        framed.extend_from_slice(&compressed);
        let code = format!("{SHARE_PREFIX}{}", BASE64_URL_SAFE_NO_PAD.encode(framed));

        assert_eq!(
            decode_and_decompress(&code).unwrap_err(),
            ShareError::PayloadTooLarge
        );
    }

    #[test]
    fn test_decoder_never_panics_on_garbage() {
        let corpus = [
            "",
            "SYN1:",
            "SYN1:AAAA",
            "SYN1:!!!!",
            "PH1:abcd",
            "not a code",
            "{\"rules\":[]}",
            "[]",
            "SYN1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ];
        for input in corpus {
            let _ = decode_and_decompress(input);
        }
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_+/=";
        let mut seed: u64 = 0x9E3779B97F4A7C15;
        for _ in 0..200 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let len = (seed % 64) as usize;
            let mut body = String::new();
            for _ in 0..len {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                body.push(alphabet[(seed % alphabet.len() as u64) as usize] as char);
            }
            let _ = decode_and_decompress(&format!("{SHARE_PREFIX}{body}"));
        }
    }

    #[test]
    fn test_checksum_rejects_one_bit_difference() {
        let (payload, _) = build_payload(vec![sample_rule("checksum_bit_rule")], &[], &[]);
        let encoded = encode(&payload);
        let body = encoded.strip_prefix(SHARE_PREFIX).unwrap();
        let framed = BASE64_URL_SAFE_NO_PAD.decode(body).unwrap();
        assert!(framed.len() > CHECKSUM_LEN);

        for bit in 0..CHECKSUM_LEN * 8 {
            let mut altered = framed.clone();
            altered[bit / 8] ^= 1 << (bit % 8);
            let code = format!("{SHARE_PREFIX}{}", BASE64_URL_SAFE_NO_PAD.encode(&altered));
            assert_eq!(
                decode_and_decompress(&code),
                Err(ShareError::ChecksumMismatch),
                "flipping checksum bit {bit} must be detected"
            );
        }
    }
}
