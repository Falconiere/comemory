//! `code_feedback` row CRUD: the code-side sibling of
//! [`crate::store::feedback`], keyed by the stable `(repo, path, symbol)`
//! identity rather than a `code_symbols` rowid — re-indexing purges and
//! reinserts every row of a touched file and SQLite recycles the freed
//! rowids, so a rowid key would silently re-attribute feedback history.
//!
//! `domains::learning::code_feedback`'s `resolve_identity` composes
//! [`own_identity`] and [`parent_identity`] into the chunk-to-parent walk
//! (the domain rule for *which* row a verdict should land under); this
//! module owns only the SQL text, the row mapping, and the
//! `feedback`/`feedback_events` CRUD.

use super::{
    orm,
    schema_code::{CodeSymbols, code_symbols as c},
    schema_learning::{FeedbackEvents, feedback_events as e},
};
use rusqlite::{Connection, params};
use toolu_orm::core::query_column::CommonOps;

use crate::prelude::*;

/// Stable identity of one code symbol: the `code_feedback` key.
pub(crate) struct SymbolIdentity {
    /// Repo label.
    pub(crate) repo: String,
    /// Repo-relative path.
    pub(crate) path: String,
    /// Symbol name (or `<parent>#<n>` for a cAST chunk).
    pub(crate) symbol: String,
}

/// Map one `(repo, path, symbol)` projection row into a [`SymbolIdentity`].
/// Shared by both lookups below.
fn identity_columns(r: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolIdentity> {
    Ok(SymbolIdentity {
        repo: r.get(0)?,
        path: r.get(1)?,
        symbol: r.get(2)?,
    })
}

/// The `(identity, parent_id)` of the `code_symbols` row at `id`, or `None`
/// when no such row exists (re-indexed away or never existed).
pub(crate) fn own_identity(
    conn: &Connection,
    id: i64,
) -> Result<Option<(SymbolIdentity, Option<i64>)>> {
    orm::query_optional(
        conn,
        CodeSymbols::select()
            .columns_typed(&[&c::repo, &c::path, &c::symbol, &c::parent_id])
            .filter(c::id.eq(id))
            .to_sql(),
        |r| Ok((identity_columns(r)?, r.get(3)?)),
    )
}

/// The identity of the `code_symbols` row at `parent_id`, or `None` when
/// absent (a raced re-index delete can vanish a chunk's parent row).
pub(crate) fn parent_identity(conn: &Connection, parent_id: i64) -> Result<Option<SymbolIdentity>> {
    orm::query_optional(
        conn,
        CodeSymbols::select()
            .columns_typed(&[&c::repo, &c::path, &c::symbol])
            .filter(c::id.eq(parent_id))
            .to_sql(),
        identity_columns,
    )
}

/// Upsert the `used` side of the per-symbol counter row: insert with
/// `used_count = 1` or bump the existing count, refreshing `last_used` to
/// `now` either way. Mirrors [`crate::store::feedback::upsert_used`].
pub(crate) fn upsert_used(conn: &Connection, sym: &SymbolIdentity, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO code_feedback(repo, path, symbol, used_count, irrelevant_count, last_used)
             VALUES (?1, ?2, ?3, 1, 0, ?4)
             ON CONFLICT(repo, path, symbol)
             DO UPDATE SET used_count = used_count + 1, last_used = ?4",
        params![sym.repo, sym.path, sym.symbol, now],
    )?;
    Ok(())
}

/// Upsert the `irrelevant` side of the per-symbol counter row: insert with
/// `irrelevant_count = 1` or bump the existing count. `last_used` is left
/// untouched — a dismissal is not a use. Mirrors
/// [`crate::store::feedback::upsert_irrelevant`].
pub(crate) fn upsert_irrelevant(conn: &Connection, sym: &SymbolIdentity) -> Result<()> {
    conn.execute(
        "INSERT INTO code_feedback(repo, path, symbol, used_count, irrelevant_count)
             VALUES (?1, ?2, ?3, 0, 1)
             ON CONFLICT(repo, path, symbol)
             DO UPDATE SET irrelevant_count = irrelevant_count + 1",
        params![sym.repo, sym.path, sym.symbol],
    )?;
    Ok(())
}

/// Insert one code-tagged `feedback_events` row, text-encoding the symbol
/// rowid into the `memory_id` column (a memory-era column-name wart the
/// reader must know about — see `crate::domains::learning::code_feedback`'s module
/// doc). `provenance` is written explicitly, never left to the column
/// default, mirroring [`crate::store::feedback::insert_event`].
pub(crate) fn insert_event(
    conn: &Connection,
    query_id: &str,
    id: i64,
    verdict: &str,
    at: &str,
    target_kind: &str,
    provenance: &str,
) -> Result<()> {
    orm::execute(
        conn,
        FeedbackEvents::insert()
            .set(&e::query_id, query_id)
            .set(&e::memory_id, id.to_string())
            .set(&e::verdict, verdict)
            .set(&e::at, at)
            .set(&e::target_kind, target_kind)
            .set(&e::provenance, provenance)
            .to_sql(),
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/code_feedback.rs"]
mod tests;
