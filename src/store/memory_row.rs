//! Shared SQLite-mirror writer for a single memory row.
//!
//! Both `cli::save` and `cli::rebuild` materialise a markdown record into the
//! v0.2 mirror with byte-identical SQL (`memories` row, `memory_tags`, FTS5,
//! graph edges, code-ref anchors) so the two paths cannot drift. Save adds one
//! extra step rebuild skips — `memory_vec` for the BYO embedding, which can't
//! be regenerated from markdown. The connection may be a
//! [`rusqlite::Transaction`]; callers own the surrounding `BEGIN`/`COMMIT`.

use rusqlite::Connection;
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::graph::cross_link;
use crate::graph::edges::{self, EdgeKey};
use crate::memory::Frontmatter;
use crate::prelude::*;
use crate::store::fts;

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
pub fn insert(
    conn: &Connection,
    fm: &Frontmatter,
    body: &str,
    slug: &str,
    md_path: &str,
    tags: &[String],
) -> Result<()> {
    let created_iso = iso_format(fm.created)?;
    // Relation-edge timestamps are captured before the outgoing-edge wipe so a
    // re-save re-inserting the same relation keeps the original `created_at`:
    // `prune::low_value::superseded_rule` compares the superseded memory's
    // `last_accessed` against the edge timestamp, and a refreshed stamp would
    // re-arm the rule on every re-save of the superseder. The wipe itself only
    // clears *outgoing* edges — incoming edges (e.g. a newer memory's
    // `supersedes` pointing here) belong to their source memory and must
    // survive a re-save and a rebuild.
    let relation_stamps = relation_edge_stamps(conn, &fm.id)?;
    insert_memories_row(conn, fm, body, slug, md_path, &created_iso)?;
    let unique_tags = insert_tags(conn, &fm.id, tags)?;
    fts::index_memory(conn, &fm.id, body, &unique_tags.join(","))?;
    insert_edges(conn, fm, &unique_tags, body, &relation_stamps)?;
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
    conn.execute("DELETE FROM memory_tags WHERE memory_id = ?1", [&fm.id])?;
    conn.execute("DELETE FROM memory_fts WHERE memory_id = ?1", [&fm.id])?;
    edges::delete_outgoing(conn, "memory", &fm.id)?;
    let simhash = crate::simhash::of_body(body) as i64;
    conn.execute(
        MEMORIES_UPSERT_SQL,
        rusqlite::params![
            &fm.id,
            slug,
            fm.kind.as_str(),
            repo_opt,
            author_opt,
            fm.quality as i64,
            fm.schema as i64,
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
        .map(|t| t.as_str())
        .filter(|t| !t.is_empty() && seen.insert(*t))
        .collect();
    for tag in &unique_tags {
        conn.execute(
            "INSERT INTO memory_tags(memory_id, tag) VALUES(?1, ?2)",
            rusqlite::params![memory_id, tag],
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
    let mut stmt = conn.prepare(
        "SELECT rel, dst_id, created_at FROM edges \
          WHERE src_kind = 'memory' AND src_id = ?1 AND dst_kind = 'memory' \
            AND rel IN ('supersedes','conflicts_with','derived_from')",
    )?;
    let rows = stmt
        .query_map([memory_id], |r| {
            Ok(((r.get::<_, String>(0)?, r.get::<_, String>(1)?), r.get(2)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

/// Insert the v0.2 graph edges that accompany a saved or rebuilt memory:
/// `in_repo`, `authored_by`, `tagged`, the frontmatter relation edges
/// (`supersedes` / `conflicts_with` / `derived_from`), plus the
/// `references_file` / `references_symbol` edges harvested from the body by
/// [`cross_link::extract_and_emit`]. Relation edges that recur from a
/// previous save reuse the captured `relation_stamps` timestamp instead of
/// a fresh one (see [`relation_edge_stamps`]).
fn insert_edges(
    conn: &Connection,
    fm: &Frontmatter,
    tags: &[&str],
    body: &str,
    relation_stamps: &std::collections::HashMap<(String, String), String>,
) -> Result<()> {
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
    insert_relation_edges(conn, fm, relation_stamps)?;
    cross_link::extract_and_emit(conn, &fm.id, body)?;
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
