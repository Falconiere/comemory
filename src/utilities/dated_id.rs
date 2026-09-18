//! The shared `<prefix>-<yyyymmdd>-<8hex>` id shape.
//!
//! Day-sortable and collision-resistant: the digest covers the seed text plus
//! a nanosecond timestamp, so the same seed twice yields two ids. Two
//! contracts are built on it — `q-` retrieval query ids ([`crate::utilities::query_id`])
//! and `rr-` reranker request ids ([`crate::utilities::rerank_protocol`]) — and
//! they share one implementation so the shapes cannot drift (Binding Rule 1).

use time::OffsetDateTime;

use crate::utilities::digest::{is_lower_hex, sha256_hex};

/// Mint `<prefix>-<yyyymmdd>-<8hex>` from `seed` and `now`.
///
/// `prefix` is an ASCII literal chosen by the contract that owns the id.
pub(crate) fn dated_id(prefix: &str, seed: &str, now: OffsetDateTime) -> String {
    let mut input = Vec::with_capacity(seed.len() + 16);
    input.extend_from_slice(seed.as_bytes());
    input.extend_from_slice(&now.unix_timestamp_nanos().to_be_bytes());
    let hex = sha256_hex(&input);
    format!(
        "{prefix}-{:04}{:02}{:02}-{}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        &hex[..8]
    )
}

/// Validate the shape [`dated_id`] emits for `prefix`.
///
/// The byte slices are safe because the length and separator checks that
/// precede them pin every inspected position to ASCII.
pub(crate) fn is_valid_dated_id(s: &str, prefix: &str) -> bool {
    let head = prefix.len() + 1;
    let bytes = s.as_bytes();
    bytes.len() == head + 17
        && s.starts_with(prefix)
        && bytes[prefix.len()] == b'-'
        && bytes[head..head + 8].iter().all(u8::is_ascii_digit)
        && bytes[head + 8] == b'-'
        && is_lower_hex(&s[head + 9..], 8)
}

#[cfg(test)]
#[path = "tests/dated_id.rs"]
mod tests;
