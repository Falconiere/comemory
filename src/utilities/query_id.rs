//! The `q-<yyyymmdd>-<8hex>` retrieval-log query id: how it is minted and how
//! it is validated.
//!
//! One leaf shared by the writer (`retrieval::pipeline`, which stamps
//! `retrieval_log.query_id`) and the checkers (`comemory feedback`, the HTTP
//! feedback routes). Kept out of the learning capability (#166) so retrieval
//! does not depend on learning orchestration once #173 lands.

use time::OffsetDateTime;

use crate::utilities::dated_id::{dated_id, is_valid_dated_id};

/// What distinguishes a retrieval query id from every other
/// `<prefix>-<yyyymmdd>-<8hex>` contract built on `utilities::dated_id`.
const QUERY_ID_PREFIX: &str = "q";

/// `q-<yyyymmdd>-<8hex>`: day-sortable, collision-resistant query id
/// derived from the query text and a nanosecond timestamp. Not a content
/// hash — the same query run twice gets two distinct ids. The writer
/// side of the contract checked by [`is_valid_query_id`]; written into
/// `retrieval_log` by `retrieval::pipeline`.
pub fn generate_query_id(query: &str, now: OffsetDateTime) -> String {
    dated_id(QUERY_ID_PREFIX, query, now)
}

/// Validate the `q-<yyyymmdd>-<8hex>` query-id shape emitted by
/// [`generate_query_id`]. Shared by `comemory feedback` (reject typos
/// loudly), the HTTP feedback routes, and tests.
pub fn is_valid_query_id(s: &str) -> bool {
    is_valid_dated_id(s, QUERY_ID_PREFIX)
}

#[cfg(test)]
#[path = "tests/query_id.rs"]
mod tests;
