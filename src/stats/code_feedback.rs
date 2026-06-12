//! Per-symbol code feedback counters: `used` and `irrelevant`.
//!
//! Code-side sibling of [`crate::stats::feedback`] (read that first — the
//! shapes deliberately mirror each other). Each `code_feedback` row is
//! keyed by the **stable (repo, path, symbol) identity**, not the
//! `code_symbols` rowid: re-indexing purges + reinserts every row of a
//! touched file and SQLite recycles the freed rowids, so a rowid key would
//! silently re-attribute feedback history to whatever symbol inherits the
//! number. Callers still address symbols by rowid (the id `search-code`
//! prints); [`record_code_with_provenance`] resolves each id to its
//! identity before writing the counter row.
//!
//! Provenance lands in the shared `feedback_events` table tagged
//! `target_kind = 'code'` with the **text-encoded symbol rowid** in the
//! `memory_id` column (a memory-era column-name wart, e.g. symbol `42` →
//! `'42'`). Events deliberately keep the rowid, not the identity: they are
//! point-in-time telemetry about one query's hit list, aged out by
//! `comemory gc`, and never re-joined against `code_symbols` for ranking.
//! Readers must filter on `target_kind` before interpreting the column —
//! `eval::golden::harvest` and `eval::mine` do.
//!
//! The upsert/insert SQL intentionally parallels `feedback.rs` rather than
//! sharing a helper: the tables differ in name, key columns, and key type,
//! and a generic helper parameterized on table name would be stringly-typed
//! overkill.

use rusqlite::{Connection, OptionalExtension};
use time::OffsetDateTime;

use crate::prelude::*;
use crate::stats::sqlite::StatsDb;
use crate::store::memory_row;

/// Stable identity of one code symbol: the `code_feedback` key.
struct SymbolIdentity {
    repo: String,
    path: String,
    symbol: String,
}

/// Resolve a `code_symbols` rowid to its stable identity, or error loudly
/// naming the id when the row is gone.
///
/// This is deliberately ASYMMETRIC with the query-id check in
/// `cli::feedback` (which only warns when the id is absent from
/// `retrieval_log`): a missing query id still leaves valid verdict targets
/// to record, but a vanished symbol id leaves *nothing* to attribute the
/// verdict to — the rowid may already name an unrelated symbol (recycled by
/// a re-index purge+reinsert), so writing it anyway would be exactly the
/// misattribution the identity key exists to prevent.
fn resolve_identity(conn: &Connection, id: i64) -> Result<SymbolIdentity> {
    conn.query_row(
        "SELECT repo, path, symbol FROM code_symbols WHERE id = ?1",
        [id],
        |r| {
            Ok(SymbolIdentity {
                repo: r.get(0)?,
                path: r.get(1)?,
                symbol: r.get(2)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| {
        Error::Config(format!(
            "code feedback: symbol id {id} not found in code_symbols \
             (re-indexed away or never existed); re-run comemory search-code \
             for current ids"
        ))
    })
}

/// Upsert the `used` side of the per-symbol counter row: insert with
/// `used_count = 1` or bump the existing count, refreshing `last_used` to
/// `now` either way. Mirrors [`crate::stats::feedback::upsert_used`].
/// Composed by [`record_code_with_provenance`] so the UPSERT SQL exists
/// exactly once.
fn upsert_code_used(conn: &Connection, sym: &SymbolIdentity, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO code_feedback(repo, path, symbol, used_count, irrelevant_count, last_used)
             VALUES (?1, ?2, ?3, 1, 0, ?4)
             ON CONFLICT(repo, path, symbol)
             DO UPDATE SET used_count = used_count + 1, last_used = ?4",
        rusqlite::params![sym.repo, sym.path, sym.symbol, now],
    )?;
    Ok(())
}

/// Upsert the `irrelevant` side of the per-symbol counter row: insert with
/// `irrelevant_count = 1` or bump the existing count. `last_used` is left
/// untouched — a dismissal is not a use. Mirrors
/// [`crate::stats::feedback::upsert_irrelevant`].
fn upsert_code_irrelevant(conn: &Connection, sym: &SymbolIdentity) -> Result<()> {
    conn.execute(
        "INSERT INTO code_feedback(repo, path, symbol, used_count, irrelevant_count)
             VALUES (?1, ?2, ?3, 0, 1)
             ON CONFLICT(repo, path, symbol)
             DO UPDATE SET irrelevant_count = irrelevant_count + 1",
        rusqlite::params![sym.repo, sym.path, sym.symbol],
    )?;
    Ok(())
}

/// Insert one code-tagged `feedback_events` provenance row, text-encoding
/// the symbol id into the `memory_id` column (see the module doc for the
/// column-name wart and why events keep the rowid while counters use the
/// identity). Private helper so [`record_code_with_provenance`]'s used and
/// irrelevant loops share the INSERT SQL.
fn insert_code_event(
    conn: &Connection,
    query_id: &str,
    id: i64,
    verdict: &str,
    at: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind)
         VALUES (?1, ?2, ?3, ?4, 'code')",
        rusqlite::params![query_id, id.to_string(), verdict, at],
    )?;
    Ok(())
}

/// Record a batch of used/irrelevant code-symbol verdicts for one query in
/// a single transaction: each rowid is resolved to its stable
/// (repo, path, symbol) identity FIRST (an unknown id errors loudly — see
/// [`resolve_identity`] for the deliberate asymmetry with the query-id warn
/// path), then one code-tagged `feedback_events` row plus the matching
/// identity-keyed `code_feedback` counter upsert land together per id.
/// All-or-nothing — a failure on any id leaves both tables untouched, so
/// events and counters cannot drift. Mirrors
/// [`crate::stats::feedback::record_with_provenance`], including the
/// recorded-verbatim query-id contract and the
/// [`memory_row::iso_format`] timestamp shared with `retrieval_log.at`.
pub fn record_code_with_provenance(
    db: &mut StatsDb,
    query_id: &str,
    used: &[i64],
    irrelevant: &[i64],
) -> Result<()> {
    let now = memory_row::iso_format(OffsetDateTime::now_utc())?;
    let tx = db.conn_mut().transaction()?;
    for id in used {
        let sym = resolve_identity(&tx, *id)?;
        insert_code_event(&tx, query_id, *id, "used", &now)?;
        upsert_code_used(&tx, &sym, &now)?;
    }
    for id in irrelevant {
        let sym = resolve_identity(&tx, *id)?;
        insert_code_event(&tx, query_id, *id, "irrelevant", &now)?;
        upsert_code_irrelevant(&tx, &sym)?;
    }
    tx.commit()?;
    Ok(())
}
