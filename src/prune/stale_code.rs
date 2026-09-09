//! Ghost code-reference detection (spec Non-Goal 5: ghost code-refs only, not
//! a general stale-code engine): live memories whose pinned `references_symbol`
//! target no longer resolves against a *current* index.
//!
//! Classification is delegated to the same [`RefStatusCache`] the `context`
//! fetch path uses, so prune and fetch agree on "ghost"; a merely stale index
//! yields `Unknown`, never `Ghost`. Detection is read-only and advisory —
//! `cli::prune` surfaces the candidates without hard-deleting them.

use crate::prelude::*;
use crate::retrieval::code_ref_fetch::RefStatusCache;
use crate::retrieval::code_ref_status::RefStatus;
use crate::store::code_ref::LiveRefRow;
use crate::store::edges::REFERENCES_SYMBOL;
use crate::store::{self, Connection};

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
fn symbol_refs(conn: &Connection) -> Result<Vec<LiveRefRow>> {
    store::code_ref::for_rel_live(conn, REFERENCES_SYMBOL)
}

/// Whether `r` classifies as [`RefStatus::Ghost`]. Reuses the shared
/// [`RefStatusCache`] (the fetch path's classifier), feeding it the `<repo>`,
/// repo-relative `<path>`, and whether the symbol still resolves to a live
/// `code_symbols` row. A malformed `dst_id` (not `<repo>:<path>:<symbol>`) is
/// skipped — it is not a ghost, just unparsable.
fn is_ghost(conn: &Connection, cache: &mut RefStatusCache, r: &LiveRefRow) -> Result<bool> {
    let Some((repo, path, symbol)) = split_symbol_id(&r.dst_id) else {
        return Ok(false);
    };
    let resolved = store::code_row::symbol_row_exists(conn, repo, path, symbol)?;
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

#[cfg(test)]
#[path = "tests/stale_code.rs"]
mod tests;
