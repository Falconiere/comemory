//! Ghost code-reference detection (spec Non-Goal 5: ghost code-refs only, not
//! a general stale-code engine): live memories whose pinned `references_symbol`
//! target no longer resolves against a *current* index.
//!
//! Classification is delegated to the same [`RefStatusCache`] the `context`
//! fetch path uses, so prune and fetch agree on "ghost"; a merely stale index
//! yields `Unknown`, never `Ghost`. Detection is read-only and advisory —
//! `cli::prune` surfaces the candidates without hard-deleting them.

use rusqlite::Connection;

use crate::graph::edges::REFERENCES_SYMBOL;
use crate::prelude::*;
use crate::retrieval::code_ref_fetch::RefStatusCache;
use crate::retrieval::code_ref_status::RefStatus;

/// One memory's anchored symbol reference, as stored in `code_ref`.
struct SymbolRef {
    memory_id: String,
    dst_id: String,
    pinned_blob: Option<String>,
}

/// Ids of live memories that own at least one `references_symbol` anchor whose
/// target resolves to [`RefStatus::Ghost`] against the current index. Returns
/// the de-duplicated, sorted candidate set — the same shape
/// [`crate::prune::low_value::detect`] returns, so `cli::prune` can render and
/// (optionally) act on it uniformly.
///
/// Only symbol refs are considered (file-ref ghosts are out of scope here per
/// spec Non-Goal 5). A ref classified `fresh`, `stale`, `unpinned`, or
/// `unknown` is *not* a candidate; only a true `ghost` (current index, target
/// gone) is.
pub fn detect(conn: &Connection) -> Result<Vec<String>> {
    let refs = symbol_refs(conn)?;
    let mut cache = RefStatusCache::default();
    let mut flagged: Vec<String> = Vec::new();
    for r in refs {
        if is_ghost(conn, &mut cache, &r)? && !flagged.contains(&r.memory_id) {
            flagged.push(r.memory_id);
        }
    }
    flagged.sort();
    flagged.dedup();
    Ok(flagged)
}

/// Every anchored symbol reference attached to a live memory, ordered by id.
fn symbol_refs(conn: &Connection) -> Result<Vec<SymbolRef>> {
    let mut stmt = conn.prepare(
        "SELECT cr.memory_id, cr.dst_id, cr.pinned_blob \
           FROM code_ref cr \
           JOIN memories m ON m.id = cr.memory_id AND m.deleted_at IS NULL \
          WHERE cr.rel = ?1 \
          ORDER BY cr.memory_id, cr.dst_id",
    )?;
    let rows = stmt.query_map([REFERENCES_SYMBOL], |row| {
        Ok(SymbolRef {
            memory_id: row.get(0)?,
            dst_id: row.get(1)?,
            pinned_blob: row.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Whether `r` classifies as [`RefStatus::Ghost`]. Reuses the shared
/// [`RefStatusCache`] (the fetch path's classifier), feeding it the `<repo>`,
/// repo-relative `<path>`, and whether the symbol still resolves to a live
/// `code_symbols` row. A malformed `dst_id` (not `<repo>:<path>:<symbol>`) is
/// skipped — it is not a ghost, just unparsable.
fn is_ghost(conn: &Connection, cache: &mut RefStatusCache, r: &SymbolRef) -> Result<bool> {
    let Some((repo, path, symbol)) = split_symbol_id(&r.dst_id) else {
        return Ok(false);
    };
    let resolved = symbol_resolves(conn, repo, path, symbol)?;
    let status = cache.status(conn, repo, path, true, r.pinned_blob.as_deref(), resolved);
    Ok(status == RefStatus::Ghost)
}

/// Split a `<repo>:<path>:<symbol>` address into its three parts. `None` when
/// the address lacks the symbol segment. Mirrors the parse in
/// [`crate::retrieval::code_ref_collect`] so both read the same edge format.
fn split_symbol_id(dst_id: &str) -> Option<(&str, &str, &str)> {
    let parts: Vec<&str> = dst_id.splitn(3, ':').collect();
    match parts.as_slice() {
        [repo, path, symbol] => Some((repo, path, symbol)),
        _ => None,
    }
}

/// Whether a live `code_symbols` row exists for `(repo, path, symbol)`.
fn symbol_resolves(conn: &Connection, repo: &str, path: &str, symbol: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM code_symbols \
          WHERE repo = ?1 AND path = ?2 AND symbol = ?3",
        rusqlite::params![repo, path, symbol],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}
