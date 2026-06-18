//! Batched navigation metadata for a set of memory ids.
//!
//! [`fetch_meta`] enriches a page of `comemory search --json` hits with the
//! fields needed to navigate to each memory (path, repo, kind, slug, tags,
//! code references) in three batched queries: `memories`, `memory_tags`, and
//! the `references_file` / `references_symbol` rows in `edges`. Each
//! reference `dst_id` is already the qualified `<repo>:<path>[:<symbol>]`
//! string the frontmatter [`References`] type carries.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::graph::edges::{REFERENCES_FILE, REFERENCES_SYMBOL};
use crate::memory::References;
use crate::prelude::*;
use crate::store::qmarks;

/// Navigation metadata for one memory row, keyed by memory id in the map
/// returned by [`fetch_meta`].
pub struct MemoryMeta {
    /// `memories.md_path` as stored (relative to the data dir per the schema,
    /// though current writers store it absolute). The caller resolves it
    /// against the data dir to get an absolute path.
    pub md_path: String,
    /// Repo the memory belongs to, or `None` when the column is NULL.
    pub repo: Option<String>,
    /// Memory kind (decision|bug|convention|discovery|pattern|note).
    pub kind: String,
    /// Filename slug derived from the body.
    pub slug: String,
    /// Tag list from `memory_tags`, in row order.
    pub tags: Vec<String>,
    /// Code references harvested from the body (`references_file` /
    /// `references_symbol` edges), reusing the frontmatter [`References`]
    /// type so the JSON shape matches the markdown source of truth.
    pub references: References,
}

/// Fetch navigation metadata for every id in `ids` in three batched queries.
///
/// Returns a map keyed by memory id; ids with no live (`deleted_at IS NULL`)
/// `memories` row are simply absent from the map. An empty `ids` slice
/// short-circuits to an empty map so no malformed `IN ()` clause is ever
/// built.
pub fn fetch_meta(conn: &Connection, ids: &[&str]) -> Result<HashMap<String, MemoryMeta>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut map = fetch_rows(conn, ids)?;
    attach_tags(conn, ids, &mut map)?;
    attach_references(conn, ids, &mut map)?;
    Ok(map)
}

/// One `IN (?, ?, ...)` parameter binding for the id list. Borrowing the
/// `&str` ids directly keeps the bind list allocation-free.
fn id_params<'a>(ids: &'a [&'a str]) -> Vec<&'a dyn rusqlite::ToSql> {
    ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect()
}

/// Pull the core `memories` columns for `ids` into the seed map. Soft-deleted
/// rows are excluded so a hit that raced a delete falls back to the caller's
/// defaults rather than surfacing a tombstoned path.
fn fetch_rows(conn: &Connection, ids: &[&str]) -> Result<HashMap<String, MemoryMeta>> {
    let sql = format!(
        "SELECT id, md_path, repo, kind, slug FROM memories \
          WHERE id IN ({}) AND deleted_at IS NULL",
        qmarks(ids.len())
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(id_params(ids).as_slice(), |r| {
        Ok((
            r.get::<_, String>(0)?,
            MemoryMeta {
                md_path: r.get(1)?,
                repo: r.get(2)?,
                kind: r.get(3)?,
                slug: r.get(4)?,
                tags: Vec::new(),
                references: References::default(),
            },
        ))
    })?;
    let mut map = HashMap::new();
    for row in rows {
        let (id, meta) = row?;
        map.insert(id, meta);
    }
    Ok(map)
}

/// Append every `memory_tags` row for `ids` onto the matching map entry.
/// Tags for an id absent from `map` (soft-deleted) are dropped.
fn attach_tags(
    conn: &Connection,
    ids: &[&str],
    map: &mut HashMap<String, MemoryMeta>,
) -> Result<()> {
    let sql = format!(
        "SELECT memory_id, tag FROM memory_tags WHERE memory_id IN ({})",
        qmarks(ids.len())
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(id_params(ids).as_slice(), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (id, tag) = row?;
        if let Some(meta) = map.get_mut(&id) {
            meta.tags.push(tag);
        }
    }
    Ok(())
}

/// Append the `references_file` / `references_symbol` edge destinations for
/// `ids` onto the matching map entry's [`References`]. The edge `dst_id` is
/// already the qualified `<repo>:<path>[:<symbol>]` string, so it is stored
/// verbatim. Refs for an id absent from `map` (soft-deleted) are dropped.
fn attach_references(
    conn: &Connection,
    ids: &[&str],
    map: &mut HashMap<String, MemoryMeta>,
) -> Result<()> {
    let sql = format!(
        "SELECT src_id, rel, dst_id FROM edges \
          WHERE src_kind = 'memory' AND rel IN (?, ?) AND src_id IN ({})",
        qmarks(ids.len())
    );
    let mut params: Vec<&dyn rusqlite::ToSql> = vec![&REFERENCES_FILE, &REFERENCES_SYMBOL];
    params.extend(id_params(ids));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params.as_slice(), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (id, rel, dst_id) = row?;
        let Some(meta) = map.get_mut(&id) else {
            continue;
        };
        if rel == REFERENCES_FILE {
            meta.references.files.push(dst_id);
        } else if rel == REFERENCES_SYMBOL {
            meta.references.symbols.push(dst_id);
        }
        // Any other rel is ignored: the WHERE clause only selects the two
        // reference kinds today, but matching explicitly keeps a future
        // memory→* rel from silently landing in `symbols`.
    }
    Ok(())
}
