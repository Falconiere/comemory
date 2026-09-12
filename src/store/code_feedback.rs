//! `code_feedback` row CRUD: the code-side sibling of
//! [`crate::store::feedback`], keyed by the stable `(repo, path, symbol)`
//! identity rather than a `code_symbols` rowid — re-indexing purges and
//! reinserts every row of a touched file and SQLite recycles the freed
//! rowids, so a rowid key would silently re-attribute feedback history.
//!
//! [`crate::stats::code_feedback::resolve_identity`] composes
//! [`own_identity`] and [`parent_identity`] into the chunk-to-parent walk
//! (the domain rule for *which* row a verdict should land under); this
//! module owns only the SQL text, the row mapping, and the
//! `feedback`/`feedback_events` CRUD.

use rusqlite::{Connection, OptionalExtension, params};

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
    let row = conn
        .query_row(
            "SELECT repo, path, symbol, parent_id FROM code_symbols WHERE id = ?1",
            [id],
            |r| Ok((identity_columns(r)?, r.get::<_, Option<i64>>(3)?)),
        )
        .optional()?;
    Ok(row)
}

/// The identity of the `code_symbols` row at `parent_id`, or `None` when
/// absent (a raced re-index delete can vanish a chunk's parent row).
pub(crate) fn parent_identity(conn: &Connection, parent_id: i64) -> Result<Option<SymbolIdentity>> {
    let row = conn
        .query_row(
            "SELECT repo, path, symbol FROM code_symbols WHERE id = ?1",
            [parent_id],
            identity_columns,
        )
        .optional()?;
    Ok(row)
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
/// reader must know about — see `crate::stats::code_feedback`'s module
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
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind, provenance)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            query_id,
            id.to_string(),
            verdict,
            at,
            target_kind,
            provenance
        ],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/code_feedback.rs"]
mod tests;
