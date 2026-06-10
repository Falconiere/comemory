//! Shared SQLite-mirror writer for a single memory row.
//!
//! Both `cli::save` and `cli::rebuild` need to materialise a markdown record
//! into the v0.2 SQLite mirror with byte-identical SQL: the `memories` row,
//! every `memory_tags` row, the FTS5 index entry, and the graph edges
//! (`in_repo`, `authored_by`, `tagged`, the frontmatter relation edges
//! `supersedes` / `conflicts_with` / `derived_from`, plus the
//! `references_file` / `references_symbol` edges harvested from the body by
//! [`cross_link`]).
//!
//! Save adds one extra step (`memory_vec` for the caller-supplied embedding)
//! that rebuild deliberately skips — vectors are BYO and cannot be
//! regenerated from markdown alone. Everything else lives here so the two
//! command paths cannot drift.
//!
//! The connection passed in may be a [`rusqlite::Transaction`] (deref's to
//! `Connection`); callers own the surrounding `BEGIN`/`COMMIT`.

use rusqlite::Connection;
use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;

use crate::graph::cross_link;
use crate::graph::edges::{self, EdgeKey};
use crate::memory::Frontmatter;
use crate::prelude::*;
use crate::store::fts;

/// Insert one parsed memory record into the v0.2 SQLite mirror.
///
/// Writes the `memories` row (using `slug` and `md_path` supplied by the
/// caller so save can reuse the already-computed values from `MemoryRecord`
/// while rebuild recomputes them from the on-disk file), every
/// `memory_tags` entry, the `memory_fts` row, and the v0.2 graph edges
/// (`in_repo` / `authored_by` / `tagged`, the frontmatter relation edges,
/// plus cross-link references parsed from the body).
///
/// The vector branch is *not* handled here — `cli::save` inserts the
/// optional `memory_vec` row inline after this helper returns. Rebuild
/// intentionally skips that branch because vectors are caller-supplied
/// (BYO-vector) and cannot be reconstructed from markdown alone.
pub fn insert(
    conn: &Connection,
    fm: &Frontmatter,
    body: &str,
    slug: &str,
    md_path: &str,
    tags: &[String],
) -> Result<()> {
    let created_iso = iso_format(fm.created)?;
    let repo_opt: Option<&str> = if fm.repo.is_empty() {
        None
    } else {
        Some(fm.repo.as_str())
    };
    let author_opt: Option<&str> = if fm.author.is_empty() {
        None
    } else {
        Some(fm.author.as_str())
    };

    // Upsert semantics: re-saving the same `id` (content_hash is the id seed,
    // so this happens when the body hasn't changed) must not blow up with a PK
    // conflict — markdown is the source of truth and a re-save just refreshes
    // the metadata. ON CONFLICT preserves the original `created_at` and bumps
    // `updated_at` to the new timestamp. The `memory_tags`, `memory_fts`, and
    // `edges` rows are wiped first so the refresh is clean rather than
    // additive (stale tag rows from a previous save can't survive a tag list
    // change, and the FTS row's UNINDEXED `memory_id` would otherwise pile up
    // duplicates).
    conn.execute(
        "DELETE FROM memory_tags WHERE memory_id = ?1",
        rusqlite::params![&fm.id],
    )?;
    conn.execute(
        "DELETE FROM memory_fts WHERE memory_id = ?1",
        rusqlite::params![&fm.id],
    )?;
    // Only *outgoing* edges are wiped: this memory re-emits its own
    // in_repo/authored_by/tagged/relation/cross-link edges below, but
    // incoming edges (e.g. a newer memory's `supersedes` pointing here)
    // belong to their source memory and must survive a re-save — and a
    // rebuild, which replays memories newest-first, so the superseder's
    // edge lands before the superseded memory is inserted.
    //
    // Relation-edge timestamps are captured before the wipe so a re-save
    // re-inserting the same relation keeps the original `created_at`:
    // `prune::low_value::superseded_rule` compares the superseded memory's
    // `last_accessed` against the edge timestamp, and a refreshed stamp
    // would re-arm the rule on every re-save of the superseder.
    let relation_stamps = relation_edge_stamps(conn, &fm.id)?;
    edges::delete_outgoing(conn, "memory", &fm.id)?;
    // Simhash is persisted at write time so the rank/diversify layers never
    // see the `DEFAULT 0` placeholder from migration 0004 on fresh saves.
    // The upsert arm refreshes it too: a re-save with a changed body must
    // not keep a stale fingerprint.
    let simhash = crate::simhash::of_body(body) as i64;
    conn.execute(
        "INSERT INTO memories(\
             id, slug, kind, repo, author, quality, schema, \
             content_hash, body, created_at, updated_at, md_path, simhash) \
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) \
         ON CONFLICT(id) DO UPDATE SET \
             slug         = excluded.slug, \
             kind         = excluded.kind, \
             repo         = excluded.repo, \
             author       = excluded.author, \
             quality      = excluded.quality, \
             schema       = excluded.schema, \
             content_hash = excluded.content_hash, \
             body         = excluded.body, \
             updated_at   = strftime('%Y-%m-%dT%H:%M:%fZ','now'), \
             md_path      = excluded.md_path, \
             simhash      = excluded.simhash, \
             deleted_at   = NULL",
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
            &created_iso,
            &created_iso,
            md_path,
            simhash,
        ],
    )?;
    // Defense-in-depth: `memory_tags` has `PRIMARY KEY (memory_id, tag)`, so a
    // duplicate entry would abort the whole transaction. Save already de-dupes
    // its `--tags` argument upstream, but rebuild feeds us `fm.tags` straight
    // from a markdown file that a human may have hand-edited with repeats. The
    // dedup here keeps the helper safe for any caller without requiring every
    // entry point to remember the constraint.
    let mut seen = std::collections::HashSet::new();
    let unique_tags: Vec<&str> = tags
        .iter()
        .map(|t| t.as_str())
        .filter(|t| !t.is_empty() && seen.insert(*t))
        .collect();
    for tag in &unique_tags {
        conn.execute(
            "INSERT INTO memory_tags(memory_id, tag) VALUES(?1, ?2)",
            rusqlite::params![&fm.id, tag],
        )?;
    }
    fts::index_memory(conn, &fm.id, body, &unique_tags.join(","))?;
    insert_edges(conn, fm, &unique_tags, body, &relation_stamps)?;
    Ok(())
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
    // Frontmatter relations become memory→memory edges so their consumers
    // (rerank's supersede penalty, prune's superseded-and-forgotten rule,
    // `edges::supersedes_chain`) actually see them. Targets are allowed to
    // dangle — same stance as cross-link refs: the dst memory may not exist
    // yet (or was hand-cited), and every reader JOINs on live `memories`
    // rows, so a dangling edge is inert until the target lands.
    for (rel, ids) in [
        ("supersedes", &fm.relations.supersedes),
        ("conflicts_with", &fm.relations.conflicts_with),
        ("derived_from", &fm.relations.derived_from),
    ] {
        for dst_id in ids {
            // Self-referential relations are skipped: a `supersedes` self-edge
            // would mark the memory as superseded by itself (permanent rank
            // penalty + prune flag). `cli::save` rejects this up front; the
            // guard here protects rebuild from hand-edited markdown carrying
            // the cycle.
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
    cross_link::extract_and_emit(conn, &fm.id, body)?;
    Ok(())
}

/// Format an [`OffsetDateTime`] as ISO-8601 for storage in the
/// `memories.created_at` / `updated_at` columns. Centralised here so save
/// and rebuild produce identical strings.
pub fn iso_format(t: OffsetDateTime) -> Result<String> {
    t.format(&Iso8601::DEFAULT)
        .map_err(|e| Error::Other(format!("iso8601 format: {e}")))
}
