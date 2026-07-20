//! Per-memory feedback counters: `used` and `irrelevant`.
//!
//! Each record corresponds to one memory id and tracks how many times the
//! memory was surfaced and accepted vs. dismissed. Inserts use SQLite UPSERT
//! so callers do not need to seed rows. The query-id contract (generate +
//! validate) lives here too so the writer and the checker cannot drift.

use rusqlite::Connection;
use time::OffsetDateTime;

use crate::prelude::*;
use crate::stats::sqlite::StatsDb;
use crate::store::memory_row;

/// `provenance` tag for implicit `used` feedback minted by the
/// co-activation reward (commits touching a memory's referenced files).
/// Distinguishes auto-reinforcement rows from the `'manual'` default
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

/// Upsert the `used` side of the per-memory counter row: insert with
/// `used_count = 1` or bump the existing count, refreshing `last_used`
/// to `now` either way. Composed by [`record_with_provenance`] so the
/// UPSERT SQL exists exactly once. Accepts any [`Connection`] (a
/// `rusqlite::Transaction` derefs to one).
pub(crate) fn upsert_used(conn: &Connection, id: &str, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback(memory_id, used_count, irrelevant_count, last_used)
             VALUES (?1, 1, 0, ?2)
             ON CONFLICT(memory_id) DO UPDATE SET used_count = used_count + 1, last_used = ?2",
        rusqlite::params![id, now],
    )?;
    Ok(())
}

/// Upsert the `irrelevant` side of the per-memory counter row: insert with
/// `irrelevant_count = 1` or bump the existing count. `last_used` is left
/// untouched — a dismissal is not a use. Composed by
/// [`record_with_provenance`].
pub(crate) fn upsert_irrelevant(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback(memory_id, used_count, irrelevant_count)
             VALUES (?1, 0, 1)
             ON CONFLICT(memory_id) DO UPDATE SET irrelevant_count = irrelevant_count + 1",
        rusqlite::params![id],
    )?;
    Ok(())
}

/// Insert one memory-tagged `feedback_events` provenance row. Private
/// helper so [`record_with_provenance`]'s used and irrelevant loops share
/// the INSERT SQL. `target_kind` is written explicitly (not left to the
/// column default) now that code-tagged rows exist too — see
/// [`crate::stats::code_feedback`] for the code-side writer.
fn insert_event(
    conn: &Connection,
    query_id: &str,
    id: &str,
    verdict: &str,
    at: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![query_id, id, verdict, at, crate::stats::target::MEMORY],
    )?;
    Ok(())
}

/// Mint one implicit `used` for `id` with caller-chosen `provenance` and
/// sentinel `query_id`: bumps the `feedback` counter via [`upsert_used`] and
/// writes a memory-target `feedback_events` row. Composes inside the
/// caller's transaction (the co-activation reward runs within materialize's),
/// so it takes a bare [`Connection`] rather than a [`StatsDb`].
///
/// `at` is the run timestamp (already `iso_format`-shaped by the caller).
/// Callers pass [`PROV_AUTO_COACTIVATION`] with [`COACTIVATION_QUERY_ID`] or
/// [`PROV_AUTO_SEARCH_EDIT`] with [`SEARCH_EDIT_QUERY_ID`]. The manual
/// `comemory feedback` path keeps writing the `'manual'` default via
/// [`insert_event`].
pub(crate) fn record_implicit_used(
    conn: &Connection,
    id: &str,
    at: &str,
    provenance: &str,
    query_id: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind, provenance)
         VALUES (?1, ?2, 'used', ?3, ?4, ?5)",
        rusqlite::params![query_id, id, at, crate::stats::target::MEMORY, provenance],
    )?;
    upsert_used(conn, id, at)?;
    Ok(())
}

/// Record a batch of used/irrelevant verdicts for one query in a single
/// transaction: one `feedback_events` row per id plus the matching
/// counter upsert, written together per id. All-or-nothing — a failure
/// on any id leaves both tables untouched, so events and counters
/// cannot drift.
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
) -> Result<()> {
    let now = memory_row::iso_format(OffsetDateTime::now_utc())?;
    let tx = db.conn_mut().transaction()?;
    for id in used {
        insert_event(&tx, query_id, id, "used", &now)?;
        upsert_used(&tx, id, &now)?;
    }
    for id in irrelevant {
        insert_event(&tx, query_id, id, "irrelevant", &now)?;
        upsert_irrelevant(&tx, id)?;
    }
    tx.commit()?;
    Ok(())
}
