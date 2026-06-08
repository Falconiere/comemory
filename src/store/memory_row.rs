//! Shared SQLite-mirror writer for a single memory row.
//!
//! Both `cli::save` and `cli::rebuild` need to materialise a markdown record
//! into the v0.2 SQLite mirror with byte-identical SQL: the `memories` row,
//! every `memory_tags` row, the FTS5 index entry, and the graph edges
//! (`in_repo`, `authored_by`, `tagged`, plus the `references_file` /
//! `references_symbol` edges harvested from the body by [`cross_link`]).
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
/// (`in_repo` / `authored_by` / `tagged` plus cross-link references parsed
/// from the body).
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
    edges::delete_touching(conn, "memory", &fm.id)?;
    conn.execute(
        "INSERT INTO memories(\
             id, slug, kind, repo, author, quality, schema, \
             content_hash, body, created_at, updated_at, md_path) \
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) \
         ON CONFLICT(id) DO UPDATE SET \
             slug         = excluded.slug, \
             kind         = excluded.kind, \
             repo         = excluded.repo, \
             author       = excluded.author, \
             quality      = excluded.quality, \
             schema       = excluded.schema, \
             content_hash = excluded.content_hash, \
             body         = excluded.body, \
             updated_at   = excluded.updated_at, \
             md_path      = excluded.md_path, \
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
        ],
    )?;
    for tag in tags {
        conn.execute(
            "INSERT INTO memory_tags(memory_id, tag) VALUES(?1, ?2)",
            rusqlite::params![&fm.id, tag],
        )?;
    }
    fts::index_memory(conn, &fm.id, body, &tags.join(","))?;
    insert_edges(conn, fm, tags, body)?;
    Ok(())
}

/// Insert the v0.2 graph edges that accompany a saved or rebuilt memory:
/// `in_repo`, `authored_by`, `tagged`, plus the `references_file` /
/// `references_symbol` edges harvested from the body by
/// [`cross_link::extract_and_emit`].
fn insert_edges(conn: &Connection, fm: &Frontmatter, tags: &[String], body: &str) -> Result<()> {
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
