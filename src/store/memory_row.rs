//! Shared SQLite-mirror writer for a single memory row.
//!
//! Both `cli::save` and `cli::rebuild` materialise a markdown record into the
//! v0.2 mirror with byte-identical SQL (`memories` row, `memory_tags`, FTS5,
//! graph edges, code-ref anchors) so the two paths cannot drift. Save adds one
//! extra step rebuild skips — `memory_vec` for the BYO embedding, which can't
//! be regenerated from markdown. The connection may be a
//! [`rusqlite::Transaction`]; callers own the surrounding `BEGIN`/`COMMIT`.
//! Rows are written here, never derived: the graph links a body owes arrive
//! as [`MemoryLinks`], resolved by [`crate::domains::memories::mirror`].

use rusqlite::Connection;
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use super::{
    orm,
    schema_graph::{Edges, edges as edge_columns},
    schema_memory::{Memories, MemoryFts, MemoryTags, memories, memory_fts, memory_tags},
};
use crate::domains::memories::Frontmatter;
use crate::prelude::*;
use crate::store::MemoryLinks;
use crate::store::edges::{self, CO_ACTIVATED, EdgeKey};
use crate::store::fts;
use toolu_orm::core::query_column::CommonOps;

/// Upsert SQL for the `memories` row. `ON CONFLICT(id)` preserves `created_at`
/// and bumps `updated_at`, so a re-save (same id, possibly changed body)
/// refreshes metadata without a PK conflict.
const MEMORIES_UPSERT_SQL: &str = "INSERT INTO memories(\
     id, slug, kind, repo, author, quality, schema, \
     content_hash, body, created_at, updated_at, md_path, simhash) \
 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) \
 ON CONFLICT(id) DO UPDATE SET \
     slug = excluded.slug, kind = excluded.kind, repo = excluded.repo, \
     author = excluded.author, quality = excluded.quality, \
     schema = excluded.schema, content_hash = excluded.content_hash, \
     body = excluded.body, \
     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'), \
     md_path = excluded.md_path, simhash = excluded.simhash, \
     deleted_at = NULL";

/// Insert one parsed memory record into the v0.2 SQLite mirror: the
/// `memories` row, `memory_tags`, the `memory_fts` entry, the graph edges,
/// and the code-ref anchors. `slug` / `md_path` are caller-supplied so save
/// reuses `MemoryRecord`'s values while rebuild recomputes them. The optional
/// `memory_vec` row is *not* handled here — `cli::save` inserts it inline
/// after this returns; rebuild skips it (BYO vectors can't be regenerated).
/// `links` carries the reference targets the caller already derived from
/// `body`; an empty [`MemoryLinks`] writes no reference edge.
pub fn insert(
    conn: &Connection,
    fm: &Frontmatter,
    body: &str,
    slug: &str,
    md_path: &str,
    tags: &[String],
    links: &MemoryLinks<'_>,
) -> Result<()> {
    let created_iso = iso_format(fm.created)?;
    // Relation-edge timestamps are captured before the outgoing-edge wipe so a
    // re-save re-inserting the same relation keeps the original `created_at`:
    // `maintenance::retention::low_value::superseded_rule` compares the
    // superseded memory's
    // `last_accessed` against the edge timestamp, and a refreshed stamp would
    // re-arm the rule on every re-save of the superseder. The wipe itself only
    // clears *outgoing* edges — incoming edges (e.g. a newer memory's
    // `supersedes` pointing here) belong to their source memory and must
    // survive a re-save and a rebuild. The mined `co_activated` edges are the
    // one outgoing kind the markdown cannot re-derive, so they are captured
    // the same way and put back after the re-materialization.
    let relation_stamps = relation_edge_stamps(conn, &fm.id)?;
    let mined = mined_edges(conn, &fm.id)?;
    insert_memories_row(conn, fm, body, slug, md_path, &created_iso)?;
    let unique_tags = insert_tags(conn, &fm.id, tags)?;
    fts::index_memory(conn, &fm.id, body, &unique_tags.join(","))?;
    insert_edges(conn, fm, &unique_tags, links, &relation_stamps)?;
    restore_mined_edges(conn, &fm.id, &mined)?;
    crate::store::code_ref::materialize(conn, &fm.id, &fm.references, &created_iso)?;
    Ok(())
}

/// Wipe this memory's `memory_tags` / `memory_fts` / outgoing-edge rows, then
/// upsert the `memories` row ([`MEMORIES_UPSERT_SQL`]). The wipe keeps the
/// refresh clean rather than additive; simhash is persisted here so the
/// rank/diversify layers never see migration 0004's `DEFAULT 0` placeholder.
fn insert_memories_row(
    conn: &Connection,
    fm: &Frontmatter,
    body: &str,
    slug: &str,
    md_path: &str,
    created_iso: &str,
) -> Result<()> {
    let repo_opt: Option<&str> = (!fm.repo.is_empty()).then_some(fm.repo.as_str());
    let author_opt: Option<&str> = (!fm.author.is_empty()).then_some(fm.author.as_str());
    orm::execute(
        conn,
        MemoryTags::delete()
            .filter(memory_tags::memory_id.eq(fm.id.as_str()))
            .to_sql(),
    )?;
    orm::execute(
        conn,
        MemoryFts::delete()
            .filter(memory_fts::memory_id.eq(fm.id.as_str()))
            .to_sql(),
    )?;
    edges::delete_outgoing(conn, "memory", &fm.id)?;
    let simhash = crate::utilities::simhash::of_body(body) as i64;
    conn.execute(
        MEMORIES_UPSERT_SQL,
        rusqlite::params![
            &fm.id,
            slug,
            fm.kind.as_str(),
            repo_opt,
            author_opt,
            i64::from(fm.quality),
            i64::from(fm.schema),
            &fm.content_hash,
            body,
            created_iso,
            created_iso,
            md_path,
            simhash,
        ],
    )?;
    Ok(())
}

/// De-dupe `tags`, insert one `memory_tags` row per unique non-empty tag, and
/// return the unique list (preserving first-seen order) for reuse by the FTS
/// index and edge emit.
///
/// Defense-in-depth: `memory_tags` has `PRIMARY KEY (memory_id, tag)`, so a
/// duplicate entry would abort the transaction. Save de-dupes its `--tags`
/// upstream, but rebuild feeds `fm.tags` straight from hand-editable markdown
/// that may carry repeats; the dedup here keeps the helper safe for any caller.
fn insert_tags<'a>(conn: &Connection, memory_id: &str, tags: &'a [String]) -> Result<Vec<&'a str>> {
    let mut seen = std::collections::HashSet::new();
    let unique_tags: Vec<&str> = tags
        .iter()
        .map(std::string::String::as_str)
        .filter(|t| !t.is_empty() && seen.insert(*t))
        .collect();
    for tag in &unique_tags {
        orm::execute(
            conn,
            MemoryTags::insert()
                .set(&memory_tags::memory_id, memory_id)
                .set(&memory_tags::tag, *tag)
                .to_sql(),
        )?;
    }
    Ok(unique_tags)
}

/// Capture `(rel, dst_id) → created_at` for a memory's existing outgoing
/// relation edges (`supersedes` / `conflicts_with` / `derived_from`).
/// Called by [`insert`] *before* the outgoing-edge wipe so re-inserted
/// relation edges can keep their original timestamps.
fn relation_edge_stamps(
    conn: &Connection,
    memory_id: &str,
) -> Result<std::collections::HashMap<(String, String), String>> {
    let query = Edges::select()
        .columns_typed(&[
            &edge_columns::rel,
            &edge_columns::dst_id,
            &edge_columns::created_at,
        ])
        .filter(edge_columns::src_kind.eq("memory"))
        .filter(edge_columns::src_id.eq(memory_id))
        .filter(edge_columns::dst_kind.eq("memory"))
        .filter(edge_columns::rel.in_list(&[
            "supersedes".into(),
            "conflicts_with".into(),
            "derived_from".into(),
        ]));
    let rows = orm::query_all(conn, query.to_sql(), |r| {
        Ok(((r.get::<_, String>(0)?, r.get::<_, String>(1)?), r.get(2)?))
    })?;
    Ok(rows.into_iter().collect())
}

/// One earned edge carried across the outgoing wipe: a `co_activated` row
/// (memory → file, weight accumulated by [`crate::domains::graph::coactivate`]) — the
/// only memory-sourced edge kind with no markdown source. `co_changed` /
/// `imports` are file- and symbol-sourced, and `auto_search_edit` is a
/// feedback provenance rather than an edge, so today this is the whole list;
/// widen the `rel` filter in [`mined_edges`] if another mined memory-sourced
/// kind lands.
struct MinedEdge {
    dst_kind: String,
    dst_id: String,
    weight: i64,
    created_at: String,
}

/// Capture the memory's mined outgoing edges before [`insert_memories_row`]
/// wipes every outgoing row. `rebuild` re-copies them from the pre-rebuild
/// database afterwards (`maintenance::rebuild::copy`), but the in-place re-mirror
/// seams (`domains::memories::update::mirror_record` — metadata PATCH, references refresh,
/// restore) have no such re-copy, so without this capture every one of them
/// would silently drop the reward.
fn mined_edges(conn: &Connection, memory_id: &str) -> Result<Vec<MinedEdge>> {
    let query = Edges::select()
        .columns_typed(&[
            &edge_columns::dst_kind,
            &edge_columns::dst_id,
            &edge_columns::weight,
            &edge_columns::created_at,
        ])
        .filter(edge_columns::src_kind.eq("memory"))
        .filter(edge_columns::src_id.eq(memory_id))
        .filter(edge_columns::rel.eq(CO_ACTIVATED));
    orm::query_all(conn, query.to_sql(), |r| {
        Ok(MinedEdge {
            dst_kind: r.get(0)?,
            dst_id: r.get(1)?,
            weight: r.get(2)?,
            created_at: r.get(3)?,
        })
    })
}

/// Put the captured mined edges back — weight and `created_at` intact — once
/// the markdown-derived edges have been re-emitted. `OR IGNORE` because the
/// wipe leaves nothing to collide with; it only guards a future writer that
/// re-emits the same key during the re-materialization.
fn restore_mined_edges(conn: &Connection, memory_id: &str, mined: &[MinedEdge]) -> Result<()> {
    for e in mined {
        orm::execute(
            conn,
            Edges::insert()
                .or_ignore()
                .set(&edge_columns::src_kind, "memory")
                .set(&edge_columns::src_id, memory_id)
                .set(&edge_columns::dst_kind, e.dst_kind.as_str())
                .set(&edge_columns::dst_id, e.dst_id.as_str())
                .set(&edge_columns::rel, CO_ACTIVATED)
                .set(&edge_columns::weight, e.weight)
                .set(&edge_columns::created_at, e.created_at.as_str())
                .to_sql(),
        )?;
    }
    Ok(())
}

/// Insert the v0.2 graph edges that accompany a saved or rebuilt memory:
/// `in_repo`, `authored_by`, `tagged`, the frontmatter relation edges
/// (`supersedes` / `conflicts_with` / `derived_from`), then the reference
/// edges the caller derived. Relation edges that recur from a previous save
/// reuse the captured `relation_stamps` timestamp instead of a fresh one
/// (see [`relation_edge_stamps`]).
fn insert_edges(
    conn: &Connection,
    fm: &Frontmatter,
    tags: &[&str],
    links: &MemoryLinks<'_>,
    relation_stamps: &std::collections::HashMap<(String, String), String>,
) -> Result<()> {
    insert_scope_edges(conn, fm, tags)?;
    insert_relation_edges(conn, fm, relation_stamps)?;
    edges::insert_memory_references(conn, &fm.id, links)
}

/// Insert the `in_repo` / `authored_by` / `tagged` edges for one memory.
fn insert_scope_edges(conn: &Connection, fm: &Frontmatter, tags: &[&str]) -> Result<()> {
    if !fm.repo.is_empty() {
        edges::insert(
            conn,
            EdgeKey {
                src_kind: "memory",
                src_id: &fm.id,
                dst_kind: "repo",
                dst_id: &fm.repo,
                rel: "in_repo",
            },
        )?;
    }
    if !fm.author.is_empty() {
        edges::insert(
            conn,
            EdgeKey {
                src_kind: "memory",
                src_id: &fm.id,
                dst_kind: "author",
                dst_id: &fm.author,
                rel: "authored_by",
            },
        )?;
    }
    for tag in tags {
        edges::insert(
            conn,
            EdgeKey {
                src_kind: "memory",
                src_id: &fm.id,
                dst_kind: "tag",
                dst_id: tag,
                rel: "tagged",
            },
        )?;
    }
    Ok(())
}

/// Emit the frontmatter relation edges (`supersedes` / `conflicts_with` /
/// `derived_from`) as memory→memory rows for their consumers (rerank, prune,
/// `edges::supersedes_chain`). Targets may dangle — readers JOIN on live
/// `memories` rows. Recurring edges reuse the captured `relation_stamps`
/// timestamp; a self-referential relation is skipped (it would mark the
/// memory superseded by itself), guarding rebuild from hand-edited cycles.
fn insert_relation_edges(
    conn: &Connection,
    fm: &Frontmatter,
    relation_stamps: &std::collections::HashMap<(String, String), String>,
) -> Result<()> {
    for (rel, ids) in [
        ("supersedes", &fm.relations.supersedes),
        ("conflicts_with", &fm.relations.conflicts_with),
        ("derived_from", &fm.relations.derived_from),
    ] {
        for dst_id in ids {
            if dst_id == &fm.id {
                tracing::warn!(
                    memory_id = %fm.id,
                    rel,
                    "skipping self-referential relation edge from frontmatter"
                );
                continue;
            }
            let stamp = relation_stamps.get(&(rel.to_string(), dst_id.clone()));
            edges::insert_at(
                conn,
                EdgeKey {
                    src_kind: "memory",
                    src_id: &fm.id,
                    dst_kind: "memory",
                    dst_id,
                    rel,
                },
                stamp.map(String::as_str),
            )?;
        }
    }
    Ok(())
}

/// Format an [`OffsetDateTime`] as ISO-8601 for storage in the
/// `memories.created_at` / `updated_at` columns. Centralised here so save
/// and rebuild produce identical strings.
pub fn iso_format(t: OffsetDateTime) -> Result<String> {
    t.format(&Iso8601::DEFAULT)
        .map_err(|e| Error::Other(format!("iso8601 format: {e}")))
}

/// Every live (`deleted_at IS NULL`) memory id, sorted ascending — the
/// deterministic dense-index mapping `graph::pagerank` needs. See
/// [`crate::domains::graph::memory_rank::derive_memory_graph`].
pub(crate) fn live_ids(conn: &Connection) -> Result<Vec<String>> {
    let query = Memories::select()
        .columns_typed(&[&memories::id])
        .filter(memories::deleted_at.is_null())
        .order_by(memories::id.asc());
    orm::query_all(conn, query.to_sql(), |r| r.get(0))
}

/// Every live memory's `(id, body)`, ordered by id so a re-embed run is
/// reproducible. Behind `maintenance::reembed`'s memory leg.
pub fn live_bodies(conn: &Connection) -> Result<Vec<(String, String)>> {
    let query = Memories::select()
        .columns_typed(&[&memories::id, &memories::body])
        .filter(memories::deleted_at.is_null())
        .order_by(memories::id.asc());
    orm::query_all(conn, query.to_sql(), |r| Ok((r.get(0)?, r.get(1)?)))
}

#[cfg(test)]
#[path = "tests/memory_row.rs"]
mod tests;
