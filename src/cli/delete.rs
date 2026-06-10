//! `comemory delete` — soft-delete a memory by id (moves the file into
//! `memories/.trash/`, stamps `deleted_at` in `comemory.db`, and removes
//! all touching graph edges + the FTS5 row).

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;
use time::OffsetDateTime;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::edges;
use crate::memory::MemoryStore;
use crate::prelude::*;
use crate::store::{connection, memory_row};

const EXAMPLES: &str = "\
Examples:
  # Soft-delete by id (moves to memories/.trash/)
  comemory delete a1b2c3d4

  # JSON output for scripting
  comemory delete a1b2c3d4 --json";

/// Arguments to `comemory delete`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// 8-hex memory id to delete.
    pub id: String,
}

/// JSON shape emitted under `--json`.
#[derive(Serialize)]
struct Output {
    deleted: String,
}

/// Soft-delete one memory: move the markdown file into `memories/.trash/`
/// (source of truth), then mirror the delete into `comemory.db` in one
/// transaction (stamp `deleted_at`, drop the `memory_fts` + `memory_vec`
/// rows, remove all touching edges). Returns the canonical id from the
/// removed record's frontmatter.
///
/// Shared by `comemory delete` and `comemory prune` (low-value apply) so
/// the two soft-delete surfaces cannot drift.
pub(crate) fn soft_delete(
    paths: &Paths,
    conn: &mut rusqlite::Connection,
    id: &str,
) -> Result<String> {
    let removed = MemoryStore::new(paths.clone()).delete(id)?;
    let id = removed.frontmatter.id;

    let now = memory_row::iso_format(OffsetDateTime::now_utc())?;
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE memories SET deleted_at = ?1 WHERE id = ?2",
        rusqlite::params![now, id],
    )?;
    tx.execute(
        "DELETE FROM memory_fts WHERE memory_id = ?1",
        rusqlite::params![id],
    )?;
    // memory_vec is a vec0 vtab — no FK cascade and no JOIN-side filter on
    // `deleted_at`, so the row would survive a soft-delete and block a
    // future re-save of the same body with a PK constraint failure.
    tx.execute(
        "DELETE FROM memory_vec WHERE memory_id = ?1",
        rusqlite::params![id],
    )?;
    edges::delete_touching(&tx, "memory", &id)?;
    tx.commit()?;
    Ok(id)
}

/// Soft-delete the memory and report the affected id.
///
/// Steps:
/// 1. Move the markdown file into `memories/.trash/` (source of truth).
/// 2. In `comemory.db`, set `memories.deleted_at` and remove the
///    `memory_fts` row and all touching edges — wrapped in one transaction.
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    // `ensure_dirs` guarantees `memories/` exists before the store enumerates
    // it — without this a fresh data dir surfaces ENOENT instead of the
    // intended "memory not found" message from `MemoryStore::delete`.
    paths.ensure_dirs()?;

    let mut conn = connection::open(paths.db_path())?;
    let id = &soft_delete(&paths, &mut conn, &a.id)?;

    let mut out = std::io::stdout().lock();
    if json {
        let output = Output {
            deleted: id.clone(),
        };
        writeln!(out, "{}", serde_json::to_string(&output)?)?;
    } else {
        writeln!(out, "deleted {id}")?;
    }
    Ok(())
}
