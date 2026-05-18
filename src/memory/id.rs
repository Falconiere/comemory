use sha2::{Digest, Sha256};

/// Compute the 8-hex-char memory id: first 4 bytes of SHA-256 of `body.trim_end()`.
///
/// Stable across calls and ignores trailing whitespace so a body that gained or
/// lost a trailing newline still maps to the same id.
pub fn memory_id(body: &str) -> String {
    let trimmed = body.trim_end();
    let digest = Sha256::digest(trimmed.as_bytes());
    let mut hex = String::with_capacity(8);
    for byte in &digest[..4] {
        use std::fmt::Write as _;
        let _ = write!(hex, "{:02x}", byte);
    }
    hex
}

/// Compute the full 64-hex-char SHA-256 digest of `bytes`.
///
/// Shared by `memory::store` (content_hash) and other crate modules that need a
/// stable hex digest; lifted out of `store.rs` to avoid the duplicated helper
/// that previously lived alongside (`code_index.rs` carries its own copy that
/// will fold in once Fix C lands).
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hex, "{:02x}", byte);
    }
    hex
}
