//! `comemory delete` — soft-delete a memory by id (moves the file into
//! `memories/.trash/`, stamps `deleted_at` in `comemory.db`, and removes
//! all touching graph edges + the FTS5 row).

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use time::OffsetDateTime;

use crate::api;
use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
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

/// Soft-delete one memory: move the markdown file into `memories/.trash/`
/// (source of truth), then mirror the delete into `comemory.db` via
/// [`mirror_soft_delete`]. Returns the canonical id from the removed
/// record's frontmatter.
///
/// Shared by `comemory delete` (via `api::delete::run`) and `comemory
/// prune` (low-value apply path) so the two soft-delete surfaces cannot
/// drift.
pub(crate) fn soft_delete(
    paths: &Paths,
    conn: &mut rusqlite::Connection,
    id: &str,
) -> Result<String> {
    let removed = MemoryStore::new(paths.clone()).delete(id)?;
    let id = removed.frontmatter.id;
    mirror_soft_delete(conn, &id)?;
    Ok(id)
}

/// Mirror a soft-delete into `comemory.db` in one transaction: stamp
/// `deleted_at`, drop the `memory_fts` + `memory_vec` rows, remove all
/// touching edges.
///
/// Factored out of [`soft_delete`] so `comemory prune` can heal a
/// half-deleted memory — live DB row, markdown already gone after a crash
/// between the file move and this transaction — with no markdown move.
///
/// After the commit the memory and its edges have left the graph, so
/// [`crate::graph::derived`] refreshes both derived artifacts best-effort
/// here, not at the [`soft_delete`] call site: every soft-delete surface
/// (delete, prune apply, prune heal) then heals rank and triplets alike.
pub(crate) fn mirror_soft_delete(conn: &mut rusqlite::Connection, id: &str) -> Result<()> {
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
    edges::delete_touching(&tx, "memory", id)?;
    tx.commit()?;
    // Nothing to report it through: a delete's response is the id.
    let _stale = crate::graph::derived::refresh_derived_best_effort(conn);
    Ok(())
}

/// Soft-delete the memory and report the affected id. Parses `Args`, opens
/// the store, and delegates to [`api::delete::run`] (Binding Rule 1) —
/// shared with `DELETE /api/v1/memories/{id}`.
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    // `ensure_dirs` guarantees `memories/` exists before the store enumerates
    // it — without this a fresh data dir surfaces ENOENT instead of the
    // intended "memory not found" message from `MemoryStore::delete`.
    paths.ensure_dirs()?;

    let cfg = load_config(&paths)?;
    let mut conn = connection::open(paths.db_path())?;
    let mut ctx = api::Ctx::borrowed(&paths, &cfg, &mut conn);
    let output = api::delete::run(&mut ctx, &a.id)?;

    let mut out = std::io::stdout().lock();
    if json {
        writeln!(out, "{}", serde_json::to_string(&output)?)?;
    } else {
        writeln!(out, "deleted {}", output.deleted)?;
    }
    Ok(())
}
