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
