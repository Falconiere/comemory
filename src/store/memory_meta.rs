//! Batched and single-row `memories` metadata reads keyed by id.
//!
//! [`fetch_meta`] enriches a page of `comemory search --json` hits with the
//! fields needed to navigate to each memory (path, repo, kind, slug, tags,
//! code references). [`ids_matching_kind`], [`kind_and_body`],
//! [`rank_signals`] and [`keeper_stats`] are smaller `memories`-table reads
//! moved here from `retrieval`/`consolidate` call sites that had no other
//! table to share a file with.

use std::collections::HashMap;

use rusqlite::Connection;

use super::{
    orm,
    schema_graph::{Edges, edges},
    schema_learning::feedback,
    schema_memory::{Memories, MemoryTags, memories, memory_tags},
};
use crate::domains::memories::{Ref, References};
use crate::prelude::*;
use crate::store::edges::{REFERENCES_FILE, REFERENCES_SYMBOL};
use toolu_orm::core::query_column::CommonOps;
use toolu_orm::core::value::Value;

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
    let values: Vec<Value> = ids.iter().copied().map(Value::from).collect();
    let mut map = fetch_rows(conn, &values)?;
    attach_tags(conn, &values, &mut map)?;
    attach_references(conn, &values, &mut map)?;
    Ok(map)
}

/// Pull the core `memories` columns for `ids` into the seed map. Soft-deleted
/// rows are excluded so a hit that raced a delete falls back to the caller's
/// defaults rather than surfacing a tombstoned path.
fn fetch_rows(conn: &Connection, ids: &[Value]) -> Result<HashMap<String, MemoryMeta>> {
    let query = live_memories()
        .columns_typed(&[
            &memories::id,
            &memories::md_path,
            &memories::repo,
            &memories::kind,
            &memories::slug,
        ])
        .filter(memories::id.in_list(ids));
    let rows = orm::query_all(conn, query.to_sql(), |r| {
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
    Ok(rows.into_iter().collect())
}

/// Append every `memory_tags` row for `ids` onto the matching map entry.
/// Tags for an id absent from `map` (soft-deleted) are dropped.
fn attach_tags(
    conn: &Connection,
    ids: &[Value],
    map: &mut HashMap<String, MemoryMeta>,
) -> Result<()> {
    let query = MemoryTags::select()
        .columns_typed(&[&memory_tags::memory_id, &memory_tags::tag])
        .filter(memory_tags::memory_id.in_list(ids));
    let rows = orm::query_all(conn, query.to_sql(), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    for (id, tag) in rows {
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
    ids: &[Value],
    map: &mut HashMap<String, MemoryMeta>,
) -> Result<()> {
    let query = Edges::select()
        .columns_typed(&[&edges::src_id, &edges::rel, &edges::dst_id])
        .filter(edges::src_kind.eq("memory"))
        .filter(edges::rel.in_list(&[REFERENCES_FILE.into(), REFERENCES_SYMBOL.into()]))
        .filter(edges::src_id.in_list(ids));
    let rows = orm::query_all(conn, query.to_sql(), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    for (id, rel, dst_id) in rows {
        let Some(meta) = map.get_mut(&id) else {
            continue;
        };
        if rel == REFERENCES_FILE {
            meta.references.files.push(Ref::new(dst_id));
        } else if rel == REFERENCES_SYMBOL {
            meta.references.symbols.push(Ref::new(dst_id));
        }
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
    let query = Memories::select()
        .columns_typed(&[&memories::id])
        .filter(memories::kind.eq(kind))
        .filter(memories::id.in_list(&ids.iter().copied().map(Value::from).collect::<Vec<_>>()));
    orm::query_all(conn, query.to_sql(), |r| r.get(0))
}

/// One live memory's `(kind, body)`, or `None` when the id is missing or
/// soft-deleted. Used by `comemory context` to assemble a bundle row for
/// each matched memory id.
pub fn kind_and_body(conn: &Connection, id: &str) -> Result<Option<(String, String)>> {
    let query = live_memories()
        .columns_typed(&[&memories::kind, &memories::body])
        .filter(memories::id.eq(id));
    orm::query_optional(conn, query.to_sql(), |r| Ok((r.get(0)?, r.get(1)?)))
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
    let query = live_memories()
        .columns_typed(&[
            &memories::body,
            &memories::author,
            &memories::quality,
            &memories::created_at,
            &memories::updated_at,
            &memories::access_count,
            &memories::last_accessed,
            &memories::rank_score,
        ])
        .filter(memories::id.eq(id));
    orm::query_optional(conn, query.to_sql(), |r| {
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
    })
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
    let query = live_memories()
        .columns_typed(&[])
        .column_expr(&memories::quality.qualified(), "quality")
        .column_expr(&memories::access_count.qualified(), "access_count")
        .column_expr(
            "COALESCE(memories.last_accessed, memories.created_at)",
            "last_accessed",
        )
        .column_expr(&memories::body.qualified(), "body")
        .column_expr(&memories::simhash.qualified(), "simhash")
        .column_expr("COALESCE(feedback.used_count, 0)", "used_count")
        .column_expr("COALESCE(feedback.irrelevant_count, 0)", "irrelevant_count")
        .column_expr(&memories::rank_score.qualified(), "rank_score")
        .left_join("feedback", feedback::memory_id.equals(&memories::id))
        .filter(memories::id.eq(id));
    orm::query_optional(conn, query.to_sql(), |r| {
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
}

/// Per-memory stats behind `maintenance::consolidation::keeper`'s
/// best-keeper ordering.
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
    let query = Memories::select()
        .columns_typed(&[
            &memories::id,
            &memories::repo,
            &memories::kind,
            &memories::quality,
            &memories::access_count,
            &memories::last_accessed,
            &memories::rank_score,
        ])
        .filter(memories::id.in_list(&ids.iter().copied().map(Value::from).collect::<Vec<_>>()));
    orm::query_all(conn, query.to_sql(), |r| {
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
    })
}

/// Start a read that excludes soft-deleted memories.
fn live_memories() -> toolu_orm::query::select::SelectBuilder {
    Memories::select().filter(memories::deleted_at.is_null())
}

#[cfg(test)]
#[path = "tests/memory_meta.rs"]
mod tests;
