//! The `q-<yyyymmdd>-<8hex>` retrieval-log query id: how it is minted and how
//! it is validated.
//!
//! One leaf shared by the writer (`retrieval::pipeline`, which stamps
//! `retrieval_log.query_id`) and the checkers (`comemory feedback`, the HTTP
//! feedback routes). Kept out of the learning capability (#166) so retrieval
//! does not depend on learning orchestration once #173 lands.

use time::OffsetDateTime;

use crate::utilities::digest::{is_lower_hex, sha256_hex};

/// `q-<yyyymmdd>-<8hex>`: day-sortable, collision-resistant query id
/// derived from the query text and a nanosecond timestamp. Not a content
/// hash — the same query run twice gets two distinct ids. The writer
/// side of the contract checked by [`is_valid_query_id`]; written into
/// `retrieval_log` by `retrieval::pipeline`.
pub fn generate_query_id(query: &str, now: OffsetDateTime) -> String {
    let mut input = Vec::with_capacity(query.len() + 16);
    input.extend_from_slice(query.as_bytes());
    input.extend_from_slice(&now.unix_timestamp_nanos().to_be_bytes());
    let hex = sha256_hex(&input);
    format!(
        "q-{:04}{:02}{:02}-{}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        &hex[..8]
    )
}

/// Validate the `q-<yyyymmdd>-<8hex>` query-id shape emitted by
/// [`generate_query_id`]. Shared by `comemory feedback` (reject typos
/// loudly), the HTTP feedback routes, and tests. The 8-hex tail is checked
/// with the shared [`is_lower_hex`] primitive — the same one
/// `memory::id::is_valid_memory_id` uses, so the two id shapes cannot drift;
/// the byte slice at 11 is safe because the earlier checks pin the first 11
/// bytes to ASCII.
pub fn is_valid_query_id(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 19
        && s.starts_with("q-")
        && b[2..10].iter().all(u8::is_ascii_digit)
        && b[10] == b'-'
        && is_lower_hex(&s[11..], 8)
}

#[cfg(test)]
#[path = "tests/query_id.rs"]
mod tests;
