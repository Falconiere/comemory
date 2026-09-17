use sha2::{Digest, Sha256};

use crate::utilities::digest::is_lower_hex;

/// Number of hex characters in a memory id.
const MEMORY_ID_HEX: usize = 8;

/// Compute the 8-hex-char memory id: first 4 bytes of SHA-256 of `body.trim_end()`.
///
/// Stable across calls and ignores trailing whitespace so a body that gained or
/// lost a trailing newline still maps to the same id.
pub fn memory_id(body: &str) -> String {
    let trimmed = body.trim_end();
    let digest = Sha256::digest(trimmed.as_bytes());
    let mut hex = String::with_capacity(MEMORY_ID_HEX);
    for byte in &digest[..4] {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// True when `s` has the exact shape of a memory id produced by
/// [`memory_id`]: eight lowercase hexadecimal characters. Used to validate
/// caller-supplied id lists (e.g. `comemory save --supersedes`) before
/// anything is written to disk.
///
/// The hex shape itself is the shared primitive
/// (`crate::utilities::digest::is_lower_hex`); what counts as a memory id —
/// the width and the meaning — stays here with the rest of the content-id
/// policy.
pub fn is_valid_memory_id(s: &str) -> bool {
    is_lower_hex(s, MEMORY_ID_HEX)
}

#[cfg(test)]
#[path = "tests/id.rs"]
mod tests;
