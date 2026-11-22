//! Portable automation exchange: one compact text code carries rules together with the
//! library presets and overlays they reference.
//!
//! Wire format (`SYN1:` prefix):
//!
//! ```text
//! SYN1:base64url( sha256(payload_json)[..8] || brotli(payload_json) )
//! ```
//!
//! The 8-byte SHA-256 prefix catches truncated or altered codes (brotli itself has no
//! checksum), the decompressor is bounded so a malicious code cannot exhaust memory, and
//! imports never carry runtime counters or sender-local icon and image paths.
//!
//! The implementation is split by stage into the `format`, `payload`, `encode`, `decode`,
//! `import` and `assets` submodules; this module keeps the original flat surface.

mod assets;
mod decode;
mod encode;
mod format;
mod import;
mod payload;

pub use decode::decode_and_decompress;
pub use encode::{build_payload, compress_and_encode};
pub use format::{
    error_text, ShareError, ShareWarning, MAX_CODE_CHARS, MAX_PAYLOAD_BYTES, MAX_SHARE_OVERLAYS,
    MAX_SHARE_PRESETS, MAX_SHARE_RULES, SHARE_PREFIX, SHARE_VERSION,
};
pub use import::prepare_import;
pub use payload::{PreparedShareImport, ShareImportSummary, SharePayload};
