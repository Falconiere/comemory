//! Batched and single-row `memories` metadata reads keyed by id.
//!
//! [`fetch_meta`] enriches a page of `comemory search --json` hits with the
//! fields needed to navigate to each memory (path, repo, kind, slug, tags,
//! code references). [`ids_matching_kind`], [`kind_and_body`],
//! [`rank_signals`] and [`keeper_stats`] are smaller `memories`-table reads
//! moved here from `retrieval`/`consolidate` call sites that had no other
//! table to share a file with.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};

use crate::memory::{Ref, References};
use crate::prelude::*;
use crate::store::edges::{REFERENCES_FILE, REFERENCES_SYMBOL};
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
            meta.references.files.push(Ref::new(dst_id));
        } else if rel == REFERENCES_SYMBOL {
            meta.references.symbols.push(Ref::new(dst_id));
        }
        // Any other rel is ignored: the WHERE clause only selects the two
        // reference kinds today, but matching explicitly keeps a future
        // memory→* rel from silently landing in `symbols`.
    }
    Ok(())
}

/// Every id in `ids` whose live `memories.kind` equals `kind`, in query
/// (not caller) order — used to post-filter an ANN leg's hits, since vec0
/// cannot filter by kind inside the KNN query itself. An empty `ids` slice
/// short-circuits to an empty result.
pub fn ids_matching_kind(conn: &Connection, kind: &str, ids: &[&str]) -> Result<Vec<String>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT id FROM memories WHERE kind = ?1 AND id IN ({})",
        qmarks(ids.len())
    );
    let mut stmt = conn.prepare(&sql)?;
    let params = std::iter::once(kind).chain(ids.iter().copied());
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params), |r| r.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(rows)
}

/// One live memory's `(kind, body)`, or `None` when the id is missing or
/// soft-deleted. Used by `comemory context` to assemble a bundle row for
/// each matched memory id.
pub fn kind_and_body(conn: &Connection, id: &str) -> Result<Option<(String, String)>> {
    conn.query_row(
        "SELECT kind, body FROM memories WHERE id = ?1 AND deleted_at IS NULL",
        [id],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )
    .optional()
    .map_err(Error::from)
}

/// Single-row `memories` columns [`fetch_meta`] does not carry, behind
/// `comemory show` — body, author, quality, timestamps, access tracking, and
/// the projected memory-graph PageRank score.
pub struct ExtraFields {
    /// Full memory body, verbatim.
    pub body: String,
    /// Frontmatter author, or empty string when unset / stored as SQL `NULL`.
    pub author: String,
    /// Frontmatter quality (1..=5).
    pub quality: u8,
    /// RFC 3339 creation timestamp.
    pub created: String,
    /// RFC 3339 last-update timestamp.
    pub updated: String,
    /// Total number of times this memory has been returned by a tracked
    /// `search` / `context` run.
    pub access_count: u64,
    /// RFC 3339 timestamp of the most recent tracked access; `None` when
    /// the memory has never been accessed since it was saved.
    pub last_accessed: Option<String>,
    /// Memory-graph PageRank score (`memories.rank_score`).
    pub rank_score: f64,
}

/// Fetch [`ExtraFields`] for one live memory. `Ok(None)` for an unknown or
/// soft-deleted id.
pub fn fetch_extra(conn: &Connection, id: &str) -> Result<Option<ExtraFields>> {
    conn.query_row(
        "SELECT body, author, quality, created_at, updated_at, access_count, last_accessed, \
                rank_score \
           FROM memories WHERE id = ?1 AND deleted_at IS NULL",
        [id],
        |r| {
            let author: Option<String> = r.get(1)?;
            Ok(ExtraFields {
                body: r.get(0)?,
                author: author.unwrap_or_default(),
                quality: r.get(2)?,
                created: r.get(3)?,
                updated: r.get(4)?,
                access_count: r.get::<_, i64>(5)?.max(0) as u64,
                last_accessed: r.get(6)?,
                rank_score: r.get(7)?,
            })
        },
    )
    .optional()
    .map_err(Error::from)
}

/// Per-memory ranking signals pulled in one query behind
/// `retrieval::rerank`: row metadata plus the (optional) `feedback`
/// counters, `COALESCE`d to neutral when absent.
pub struct RankSignals {
    /// Frontmatter quality (1..=5).
    pub quality: u8,
    /// Times the memory was returned by a tracked search.
    pub access_count: u64,
    /// Last access timestamp (falls back to `created_at`).
    pub last_accessed: String,
    /// Full memory body.
    pub body: String,
    /// 64-bit SimHash of the body.
    pub simhash: u64,
    /// `feedback.used_count`, or `0` when no row exists.
    pub used: u64,
    /// `feedback.irrelevant_count`, or `0` when no row exists.
    pub irrelevant: u64,
    /// Projected memory-graph PageRank score.
    pub rank_score: f64,
}

/// Fetch the ranking signals for one live memory. Returns `Ok(None)` when
/// the row does not exist or is soft-deleted. `prepare_cached` so a
/// per-candidate rerank loop reuses one prepared statement.
pub fn rank_signals(conn: &Connection, id: &str) -> Result<Option<RankSignals>> {
    let mut stmt = conn.prepare_cached(
        "SELECT m.quality, m.access_count, COALESCE(m.last_accessed, m.created_at),
                m.body, m.simhash,
                COALESCE(f.used_count, 0), COALESCE(f.irrelevant_count, 0),
                m.rank_score
           FROM memories m
           LEFT JOIN feedback f ON f.memory_id = m.id
          WHERE m.id = ?1 AND m.deleted_at IS NULL",
    )?;
    stmt.query_row([id], |r| {
        Ok(RankSignals {
            quality: r.get(0)?,
            access_count: r.get::<_, i64>(1)?.max(0) as u64,
            last_accessed: r.get(2)?,
            body: r.get(3)?,
            simhash: r.get::<_, i64>(4)? as u64,
            used: r.get::<_, i64>(5)?.max(0) as u64,
            irrelevant: r.get::<_, i64>(6)?.max(0) as u64,
            rank_score: r.get(7)?,
        })
    })
    .optional()
    .map_err(Error::from)
}

/// Per-memory stats behind `consolidate::keeper`'s best-keeper ordering.
#[derive(Debug, Clone, Default)]
pub struct KeeperStats {
    /// Owning repo, or `None` when the memory has none.
    pub repo: Option<String>,
    /// Memory kind.
    pub kind: String,
    /// Frontmatter quality (1..=5).
    pub quality: u8,
    /// Times the memory was returned by a tracked search.
    pub access_count: i64,
    /// Last access timestamp, or `None` if never accessed.
    pub last_accessed: Option<String>,
    /// Projected memory-graph PageRank score.
    pub rank_score: f64,
}

/// Batch-fetch [`KeeperStats`] for exactly `ids`, in query order. An empty
/// `ids` slice short-circuits to an empty `Vec`.
pub fn keeper_stats(conn: &Connection, ids: &[&str]) -> Result<Vec<(String, KeeperStats)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT id, repo, kind, quality, access_count, last_accessed, rank_score \
         FROM memories WHERE id IN ({})",
        qmarks(ids.len())
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(ids.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                KeeperStats {
                    repo: r.get(1)?,
                    kind: r.get(2)?,
                    quality: r.get(3)?,
                    access_count: r.get(4)?,
                    last_accessed: r.get(5)?,
                    rank_score: r.get(6)?,
                },
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
#[path = "tests/memory_meta.rs"]
mod tests;
