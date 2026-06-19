//! Collect a memory's walked code-reference edges into resolved [`RawRef`]s.
//!
//! A reference edge (`references_file` / `references_symbol`) is paired with the
//! anchor captured at save (`code_ref.pinned_blob`) and, for symbol refs, the
//! live `code_symbols` row. The result is the unranked input that
//! [`crate::retrieval::bundle`] scores and freshness-classifies. Split out of
//! `bundle.rs` to keep that file under the module-size cap.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension};

use crate::prelude::*;

/// One walked code reference before ranking: identity, the resolved
/// `code_symbols` rowid and snippet when the address matched, plus the
/// save-time anchor and the file/symbol discriminant.
pub(crate) struct RawRef {
    /// Qualified address `<repo>:<path>[:<symbol>]`.
    pub(crate) id: String,
    pub(crate) repo: String,
    pub(crate) path: String,
    pub(crate) symbol: String,
    pub(crate) snippet: String,
    pub(crate) symbol_id: Option<i64>,
    /// First source line of the resolved symbol.
    pub(crate) line_start: Option<i64>,
    /// `true` for a symbol ref, `false` for a file ref.
    pub(crate) is_symbol: bool,
    /// Git blob OID captured at save time; `None` when the ref is unpinned.
    pub(crate) pinned_blob: Option<String>,
}

/// Load a memory's `code_ref` anchors into a `(rel, dst_id) → pinned_blob` map.
/// Refs with no captured blob map to `None` (unpinned).
pub(crate) fn anchor_map(
    conn: &Connection,
    memory_id: &str,
) -> Result<BTreeMap<(String, String), Option<String>>> {
    let mut map = BTreeMap::new();
    for row in crate::store::code_ref::for_memory(conn, memory_id)? {
        map.insert((row.rel, row.dst_id), row.pinned_blob);
    }
    Ok(map)
}

/// Build a [`RawRef`] from a walked reference edge `(rel, dst_id)`, attaching
/// the pinned blob and `is_symbol` flag. Returns `Ok(None)` for non-reference
/// edges (`relates_to` / `supersedes`) or a malformed symbol address.
pub(crate) fn ref_from_edge(
    conn: &Connection,
    rel: &str,
    dst_id: &str,
    anchors: &BTreeMap<(String, String), Option<String>>,
) -> Result<Option<RawRef>> {
    let pinned = anchors
        .get(&(rel.to_string(), dst_id.to_string()))
        .cloned()
        .flatten();
    match rel {
        "references_symbol" => symbol_ref(conn, dst_id, pinned),
        "references_file" => Ok(Some(file_ref(dst_id, pinned))),
        _ => Ok(None),
    }
}

/// Parse a `<repo>:<path>:<symbol>` edge destination and look up its
/// `code_symbols` row. Returns `Ok(None)` only when the address shape is
/// wrong (logged, skipped); a well-formed address with no matching row still
/// yields a [`RawRef`] — with no `symbol_id` and an empty snippet — so refs to
/// not-yet-indexed symbols stay visible in the bundle.
fn symbol_ref(
    conn: &Connection,
    dst_id: &str,
    pinned_blob: Option<String>,
) -> Result<Option<RawRef>> {
    let parts: Vec<&str> = dst_id.splitn(3, ':').collect();
    if parts.len() != 3 {
        tracing::warn!(
            dst_id = %dst_id,
            "malformed references_symbol edge destination (expected <repo>:<path>:<symbol>); skipping"
        );
        return Ok(None);
    }
    let (repo, path, symbol) = (parts[0], parts[1], parts[2]);
    // `prepare_cached`: this lookup runs once per walked edge inside the
    // assemble loop, so a fresh prepare per call would re-parse the SQL.
    let mut stmt = conn.prepare_cached(
        "SELECT id, snippet, line_start FROM code_symbols \
          WHERE repo = ?1 AND path = ?2 AND symbol = ?3 LIMIT 1",
    )?;
    let row = stmt
        .query_row(rusqlite::params![repo, path, symbol], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .optional()?;
    let (symbol_id, snippet, line_start) = match row {
        Some((id, snippet, line)) => (Some(id), snippet, Some(line)),
        None => (None, String::new(), None),
    };
    Ok(Some(RawRef {
        id: dst_id.to_string(),
        repo: repo.into(),
        path: path.into(),
        symbol: symbol.into(),
        snippet,
        symbol_id,
        line_start,
        is_symbol: true,
        pinned_blob,
    }))
}

/// Build a file [`RawRef`] from a `<repo>:<path>` edge destination. File refs
/// carry no symbol / snippet / line — staleness is decided purely by the
/// HEAD-tree blob compare, which is index-independent. A `dst_id` without a
/// `:` separator degrades to `path == dst_id`, repo empty.
fn file_ref(dst_id: &str, pinned_blob: Option<String>) -> RawRef {
    let (repo, path) = dst_id.split_once(':').unwrap_or(("", dst_id));
    RawRef {
        id: dst_id.to_string(),
        repo: repo.into(),
        path: path.into(),
        symbol: String::new(),
        snippet: String::new(),
        symbol_id: None,
        line_start: None,
        is_symbol: false,
        pinned_blob,
    }
}
