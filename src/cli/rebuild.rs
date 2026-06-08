//! `comemory rebuild` — drop the SQLite mirror and repopulate it from
//! the on-disk markdown files.
//!
//! Markdown remains the source of truth in v0.2; `comemory.db` is a
//! rebuildable derived cache. When the DB drifts (schema change, corruption,
//! manual deletion), `comemory rebuild` walks every `memories/*.md`, parses
//! the YAML frontmatter, and reinserts the `memories` + `memory_tags` +
//! `memory_fts` rows along with the graph edges harvested from the body.
//!
//! Vectors are intentionally *not* repopulated here: the v0.2 contract is
//! BYO-vector, so re-embedding requires running the caller's embedder
//! against the markdown bodies and piping the result through `comemory save`
//! or a future ingest command. The lexical path (`memory_fts`) is fully
//! restored, which is enough to answer the lexical branch of the router.

use std::path::PathBuf;

use clap::Args as ClapArgs;
use rusqlite::Connection;
use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::cross_link;
use crate::graph::edges::{self, EdgeKey};
use crate::memory::frontmatter::Frontmatter;
use crate::memory::slug::slug_from_body;
use crate::prelude::*;
use crate::store::{connection, fts};

/// Arguments to `comemory rebuild`. Currently no flags — the command always
/// drops the entire SQLite mirror and rebuilds from `memories/`. Wrapped in
/// a struct so future opt-in flags (e.g. `--keep-stats`, `--dry-run`) can
/// land without breaking the dispatcher signature.
#[derive(ClapArgs, Debug)]
pub struct Args;

/// Drop `comemory.db` and rebuild every row mirror-able from markdown:
/// `memories`, `memory_tags`, `memory_fts`, and the v0.2 graph edges
/// (`in_repo`, `authored_by`, `tagged`, plus cross-link references parsed
/// from the body).
pub async fn run(_args: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;

    let db = paths.db_path();
    if db.exists() {
        std::fs::remove_file(&db).map_err(Error::Io)?;
    }
    let mut conn = connection::open(&db)?;
    let tx = conn.transaction()?;

    for entry in std::fs::read_dir(paths.memories_dir()).map_err(Error::Io)? {
        let entry = entry.map_err(Error::Io)?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // Skip hidden files (`.{id}.tmp` staging files and any future
        // dotfiles in `memories/`).
        if name.starts_with('.') {
            continue;
        }
        let raw = std::fs::read_to_string(&path).map_err(Error::Io)?;
        let (fm, body) = Frontmatter::split(&raw)?;
        insert_memory(&tx, &paths, &path, &fm, &body)?;
    }

    tx.commit()?;
    Ok(())
}

/// Insert a single parsed markdown record into the SQLite mirror. Mirrors
/// the v0.2 save path in `cli::save::write_sqlite_mirror` but without the
/// optional caller-supplied vector — `rebuild` is intentionally lexical-only.
fn insert_memory(
    conn: &Connection,
    paths: &Paths,
    md_path: &std::path::Path,
    fm: &Frontmatter,
    body: &str,
) -> Result<()> {
    let created_iso = format_iso(fm.created)?;
    let slug = slug_from_body(body);
    let rel_md = md_path
        .strip_prefix(paths.data_dir())
        .unwrap_or(md_path)
        .to_string_lossy()
        .to_string();
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

    conn.execute(
        "INSERT INTO memories(\
             id, slug, kind, repo, author, quality, schema, \
             content_hash, body, created_at, updated_at, md_path) \
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        rusqlite::params![
            &fm.id,
            &slug,
            fm.kind.as_str(),
            repo_opt,
            author_opt,
            fm.quality as i64,
            fm.schema as i64,
            &fm.content_hash,
            body,
            &created_iso,
            &created_iso,
            &rel_md,
        ],
    )?;
    for tag in &fm.tags {
        conn.execute(
            "INSERT INTO memory_tags(memory_id, tag) VALUES(?1, ?2)",
            rusqlite::params![&fm.id, tag],
        )?;
    }
    fts::index_memory(conn, &fm.id, body, &fm.tags.join(","))?;
    insert_rebuild_edges(conn, fm, body)?;
    Ok(())
}

/// Insert the v0.2 graph edges that accompany a rebuilt memory: in_repo,
/// authored_by, tagged, plus references_file / references_symbol harvested
/// from the body by the cross-link parser. Mirrors `cli::save::insert_save_edges`.
fn insert_rebuild_edges(conn: &Connection, fm: &Frontmatter, body: &str) -> Result<()> {
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
    for tag in &fm.tags {
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

/// Format an `OffsetDateTime` as ISO-8601 for storage in the
/// `memories.created_at` / `updated_at` columns. Matches the formatter
/// used by `cli::save::format_iso` so rebuilds yield identical strings.
fn format_iso(t: OffsetDateTime) -> Result<String> {
    t.format(&Iso8601::DEFAULT)
        .map_err(|e| Error::Other(format!("iso8601 format: {e}")))
}
