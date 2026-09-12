//! Per-memory feedback counters: `used` and `irrelevant`.
//!
//! Each record corresponds to one memory id and tracks how many times the
//! memory was surfaced and accepted vs. dismissed. Inserts use SQLite UPSERT
//! so callers do not need to seed rows. The query-id contract (generate +
//! validate) lives here too so the writer and the checker cannot drift.

use time::OffsetDateTime;

use crate::prelude::*;
use crate::stats::sqlite::StatsDb;
use crate::store::Connection;
use crate::store::feedback as store_feedback;
use crate::store::memory_row;

/// `provenance` for a verdict a human stated: `comemory feedback`, and the
/// two HTTP feedback routes when `source` is omitted or `"explicit"`. It is
/// the column's own `DEFAULT` (`0008_v8_reinforcement.sql`), written
/// explicitly by every writer since #130 so no INSERT leans on it.
pub(crate) const PROV_MANUAL: &str = "manual";

/// `provenance` for a verdict an HTTP caller labelled `source: "implicit"`
/// — an observed use (an answer citing the memory) rather than a stated
/// one. Counted in `learning/summary`'s `implicit_share` like the `auto_*`
/// tags, and like them never harvested into the golden set nor used to mark
/// a query succeeded for reformulation mining (`store::feedback`'s readers
/// take [`PROV_MANUAL`]).
pub(crate) const PROV_IMPLICIT: &str = "implicit";

/// `provenance` tag for implicit `used` feedback minted by the
/// co-activation reward (commits touching a memory's referenced files).
/// Distinguishes auto-reinforcement rows from the [`PROV_MANUAL`] rows
/// written by `comemory feedback`. Matches the column added in
/// `0008_v8_reinforcement.sql`.
pub(crate) const PROV_AUTO_COACTIVATION: &str = "auto_coactivation";

/// `provenance` for search→edit credit: memory appeared in a recent
/// `retrieval_log` page *and* a referenced file was touched in the mined
/// commits. Still excluded from golden harvest via the sentinel query id.
pub(crate) const PROV_AUTO_SEARCH_EDIT: &str = "auto_search_edit";

/// Sentinel `query_id` stamped on co-activation `feedback_events` rows.
/// Deliberately NOT a real `q-<yyyymmdd>-<8hex>` id: `eval::golden::harvest`
/// INNER JOINs `feedback_events.query_id = retrieval_log.query_id`, and this
/// sentinel has no `retrieval_log` row, so an auto-reinforced memory can
/// never mint a golden pair — closing the confirmation loop.
pub(crate) const COACTIVATION_QUERY_ID: &str = "auto-coactivation";

/// Sentinel `query_id` for search→edit implicit `used` rows. Same golden
/// exclusion contract as [`COACTIVATION_QUERY_ID`].
pub(crate) const SEARCH_EDIT_QUERY_ID: &str = "auto-search-edit";

/// The caller-facing `source` vocabulary of `POST /api/v1/feedback` and
/// `POST /api/v1/search/{query_id}/feedback`, and its one mapping onto the
/// stored `feedback_events.provenance` value. Closed on purpose: a free
/// string would let a client spell `manual` a second way and split the
/// `= 'manual'` test every reader of the column keys on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// A human stated the verdict. Stored as [`PROV_MANUAL`].
    #[default]
    Explicit,
    /// The verdict was observed, not stated. Stored as [`PROV_IMPLICIT`].
    Implicit,
}

impl Source {
    /// Parse the exact lowercase words `explicit` / `implicit`. Anything
    /// else — `manual`, `Implicit`, an empty string — is an
    /// [`Error::BadRequest`] naming the offender (HTTP `400 bad_request`),
    /// the same shape the per-hit route answers for an unknown `signal`.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "explicit" => Ok(Self::Explicit),
            "implicit" => Ok(Self::Implicit),
            other => Err(Error::BadRequest(format!(
                "unknown source `{other}`: expected explicit or implicit"
            ))),
        }
    }

    /// The `feedback_events.provenance` value verdicts of this source are
    /// stored under.
    pub fn provenance(self) -> &'static str {
        match self {
            Self::Explicit => PROV_MANUAL,
            Self::Implicit => PROV_IMPLICIT,
        }
    }
}

/// `q-<yyyymmdd>-<8hex>`: day-sortable, collision-resistant query id
/// derived from the query text and a nanosecond timestamp. Not a content
/// hash — the same query run twice gets two distinct ids. The writer
/// side of the contract checked by [`is_valid_query_id`]; written into
/// `retrieval_log` by `retrieval::pipeline`.
pub fn generate_query_id(query: &str, now: OffsetDateTime) -> String {
    let mut input = Vec::with_capacity(query.len() + 16);
    input.extend_from_slice(query.as_bytes());
    input.extend_from_slice(&now.unix_timestamp_nanos().to_be_bytes());
    let hex = crate::memory::id::sha256_hex(&input);
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
/// loudly) and tests. The 8-hex tail has exactly the shape of a memory
/// id, so the check is delegated to
/// [`crate::memory::id::is_valid_memory_id`]; the byte slice at 11 is
/// safe because the earlier checks pin the first 11 bytes to ASCII.
pub fn is_valid_query_id(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 19
        && s.starts_with("q-")
        && b[2..10].iter().all(u8::is_ascii_digit)
        && b[10] == b'-'
        && crate::memory::id::is_valid_memory_id(&s[11..])
}

/// Mint one implicit `used` for `id` with caller-chosen `provenance` and
/// sentinel `query_id`: bumps the `feedback` counter and writes a
/// memory-target `feedback_events` row. Composes inside the caller's
/// transaction (the co-activation reward runs within materialize's), so it
/// takes a bare [`Connection`] rather than a [`StatsDb`]. The SQL lives in
/// [`store_feedback`].
///
/// `at` is the run timestamp (already `iso_format`-shaped by the caller).
/// Callers pass [`PROV_AUTO_COACTIVATION`] with [`COACTIVATION_QUERY_ID`] or
/// [`PROV_AUTO_SEARCH_EDIT`] with [`SEARCH_EDIT_QUERY_ID`]. The
/// `comemory feedback` / HTTP path is [`record_with_provenance`], which
/// writes [`PROV_MANUAL`] or [`PROV_IMPLICIT`] through the same
/// [`store_feedback::insert_event`].
pub(crate) fn record_implicit_used(
    conn: &Connection,
    id: &str,
    at: &str,
    provenance: &str,
    query_id: &str,
) -> Result<()> {
    store_feedback::insert_event(
        conn,
        query_id,
        id,
        "used",
        at,
        crate::stats::target::MEMORY,
        provenance,
    )?;
    store_feedback::upsert_used(conn, id, at)?;
    Ok(())
}

/// Record a batch of used/irrelevant verdicts for one query in a single
/// transaction: one `feedback_events` row per id plus the matching
/// counter upsert, written together per id. All-or-nothing — a failure
/// on any id leaves both tables untouched, so events and counters
/// cannot drift.
///
/// `provenance` is stamped on every event row of the batch — a
/// [`Source::provenance`] value ([`PROV_MANUAL`] / [`PROV_IMPLICIT`]) from
/// `api::feedback::run`. Both verdicts carry it, so an implicit *negative*
/// is storable, not only an implicit `used`.
///
/// The query id is recorded verbatim; it is not required to exist in
/// `retrieval_log` (gc may have evicted the row, or the caller may be
/// replaying feedback) — the caller decides whether to warn.
///
/// `feedback_events.at` goes through [`memory_row::iso_format`] — the
/// same writer as `retrieval_log.at` — so gc's lexicographic cutoff
/// comparison stays sound across both tables.
pub fn record_with_provenance(
    db: &mut StatsDb,
    query_id: &str,
    used: &[String],
    irrelevant: &[String],
    provenance: &str,
) -> Result<()> {
    let now = memory_row::iso_format(OffsetDateTime::now_utc())?;
    let tx = db.conn_mut().transaction()?;
    for id in used {
        store_feedback::insert_event(
            &tx,
            query_id,
            id,
            "used",
            &now,
            crate::stats::target::MEMORY,
            provenance,
        )?;
        store_feedback::upsert_used(&tx, id, &now)?;
    }
    for id in irrelevant {
        store_feedback::insert_event(
            &tx,
            query_id,
            id,
            "irrelevant",
            &now,
            crate::stats::target::MEMORY,
            provenance,
        )?;
        store_feedback::upsert_irrelevant(&tx, id)?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/feedback.rs"]
mod tests;
