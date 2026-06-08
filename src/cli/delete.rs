//! `comemory delete` — soft-delete a memory by id (moves the file into
//! `memories/.trash/`, stamps `deleted_at` in `comemory.db`, and removes
//! all touching graph edges + the FTS5 row).

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;
use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::edges;
use crate::memory::MemoryStore;
use crate::prelude::*;
use crate::store::connection;

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

    let removed = MemoryStore::new(paths.clone()).delete(&a.id)?;
    let id = &removed.frontmatter.id;

    // Mirror the soft-delete into comemory.db in one transaction.
    let mut conn = connection::open(paths.db_path())?;
    let now = OffsetDateTime::now_utc()
        .format(&Iso8601::DEFAULT)
        .map_err(|e| Error::Other(format!("iso8601 format: {e}")))?;
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE memories SET deleted_at = ?1 WHERE id = ?2",
        rusqlite::params![now, id],
    )?;
    tx.execute(
        "DELETE FROM memory_fts WHERE memory_id = ?1",
        rusqlite::params![id],
    )?;
    edges::delete_touching(&tx, "memory", id)?;
    tx.commit()?;

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
