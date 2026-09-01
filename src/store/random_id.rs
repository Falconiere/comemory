//! Shared random-hex id generation, read from `/dev/urandom`.
//!
//! A neutral home under `store/` so both `serve::security` (session tokens,
//! job ids) and `api::gc` (`gc_runs` row ids) draw from the one
//! implementation, instead of `api::` depending on `serve::` — a layering
//! `api::gc` must not introduce (Binding Rule 1: no duplication).

use std::fmt::Write as _;
use std::io::Read as _;

use crate::prelude::*;

/// Read `bytes` bytes from `/dev/urandom` (present on every cargo-dist
/// target — all unix) and hex-encode them (lowercase, `2 * bytes` chars).
///
/// Returns an error rather than falling back to weak entropy: every caller
/// (the session token, job ids, a `gc_runs` row id) needs the entropy, not a
/// degraded default.
pub fn random_hex(bytes: usize) -> Result<String> {
    let mut f = std::fs::File::open("/dev/urandom").map_err(Error::Io)?;
    let mut buf = vec![0u8; bytes];
    f.read_exact(&mut buf).map_err(Error::Io)?;
    let mut hex = String::with_capacity(bytes * 2);
    for b in buf {
        // Infallible: writing to a String never errors.
        let _ = write!(hex, "{b:02x}");
    }
    Ok(hex)
}

#[cfg(test)]
#[path = "tests/random_id.rs"]
mod tests;
