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
};
use rusqlite::{Connection, params};
use toolu_orm::core::query_column::CommonOps;

use crate::prelude::*;

/// Stable identity of one code symbol: the `code_feedback` key, plus the
/// content version the row was indexed at.
pub(crate) struct SymbolIdentity {
    /// Repo label.
    pub(crate) repo: String,
    /// Repo-relative path.
    pub(crate) path: String,
    /// Symbol name (or `<parent>#<n>` for a cAST chunk).
    pub(crate) symbol: String,
    /// The file's blob OID at index time — not part of the counter key, but
    /// the code version a shared verdict names (#254).
    pub(crate) version: Option<String>,
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
            .columns_typed(&[&c::repo, &c::path, &c::symbol, &c::blob_oid, &c::parent_id])
            .filter(c::id.eq(id))
            .to_sql(),
        |r| {
            Ok((
                SymbolIdentity {
                    repo: r.get(0)?,
                    path: r.get(1)?,
                    symbol: r.get(2)?,
                    version: r.get(3)?,
                },
                r.get(4)?,
            ))
        },
    )
}

/// The identity of the `code_symbols` row at `parent_id`, or `None` when
/// absent (a raced re-index delete can vanish a chunk's parent row). The same
/// lookup as [`own_identity`] with the row's own `parent_id` discarded — one
/// projection and one row mapping, so the two reads cannot disagree on what a
/// symbol's identity is.
pub(crate) fn parent_identity(conn: &Connection, parent_id: i64) -> Result<Option<SymbolIdentity>> {
    Ok(own_identity(conn, parent_id)?.map(|(identity, _)| identity))
}

/// Upsert the `used` side of the per-symbol counter row: insert with
/// `used_count = 1` or bump the existing count, refreshing `last_used` to
/// `now` either way. Mirrors [`crate::store::feedback::upsert_used`].
pub(crate) fn upsert_used(conn: &Connection, sym: &SymbolIdentity, now: &str) -> Result<()> {
    upsert_counts(conn, sym, (1, 0), Some(now))
}

/// Upsert the `irrelevant` side of the per-symbol counter row: insert with
/// `irrelevant_count = 1` or bump the existing count. `last_used` is left
/// untouched — a dismissal is not a use. Mirrors
/// [`crate::store::feedback::upsert_irrelevant`].
pub(crate) fn upsert_irrelevant(conn: &Connection, sym: &SymbolIdentity) -> Result<()> {
    upsert_counts(conn, sym, (0, 1), None)
}

/// The one `code_feedback` upsert behind [`upsert_used`] and
/// [`upsert_irrelevant`]: insert the row with `(used, irrelevant)` as its
/// counts, or add them to the existing row's. `last_used` is the later of the
/// stored and the offered stamp, ignoring a missing one on either side — `None`
/// leaves a stored stamp untouched (SQLite's scalar `MAX` is NULL if any
/// argument is, hence the `COALESCE` fallbacks).
fn upsert_counts(
    conn: &Connection,
    sym: &SymbolIdentity,
    (used, irrelevant): (i64, i64),
    last_used: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO code_feedback(repo, path, symbol, used_count, irrelevant_count, last_used)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(repo, path, symbol)
             DO UPDATE SET used_count = used_count + excluded.used_count,
                 irrelevant_count = irrelevant_count + excluded.irrelevant_count,
                 last_used = COALESCE(MAX(last_used, excluded.last_used),
                                      last_used, excluded.last_used)",
        params![sym.repo, sym.path, sym.symbol, used, irrelevant, last_used],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/code_feedback.rs"]
mod tests;
