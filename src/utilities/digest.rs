//! SHA-256 hex digests and the lowercase-hex shape check derived from them.
//!
//! These are the pure primitives shared by the two id contracts that used to
//! duplicate them: `memory::id` (the 8-hex memory id and the 64-hex
//! `content_hash`) and `utilities::query_id` (the `q-<yyyymmdd>-<8hex>`
//! retrieval-log id). The *policy* — what a memory id means, what a query id
//! means — stays with its owner; only the digest and the hex shape live here.

use sha2::{Digest, Sha256};

/// Compute the full 64-hex-char SHA-256 digest of `bytes`.
///
/// Shared by `memory::store` (`content_hash`), `api::doctor::checks`,
/// `domains::memories::save`, `domains::sync::exchange::import_state` and [`crate::utilities::query_id`],
/// so a stable hex digest is produced in exactly one place.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// True when `s` is exactly `len` lowercase hexadecimal characters.
///
/// The shape both id contracts check: `memory::id::is_valid_memory_id` calls
/// it with `8`, and [`crate::utilities::query_id::is_valid_query_id`] calls it
/// with `8` for the tail after `q-<yyyymmdd>-`. ASCII-only by construction —
/// every accepted byte is a digit or `a`..=`f`.
pub(crate) fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
#[path = "tests/digest.rs"]
mod tests;
