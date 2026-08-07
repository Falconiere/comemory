//! `api::rebuild::{Request, run}` — the shared middle of `comemory rebuild`
//! / `POST /api/v1/rebuild` (job): atomically replace the SQLite mirror,
//! preserving the code index, by rebuilding from the on-disk markdown files.
//! Moved out of `cli::rebuild::run` (Binding Rule 1).
//!
//! Markdown remains the source of truth in v0.2; `comemory.db` is a
//! rebuildable derived cache. When the DB drifts (schema change, corruption,
//! manual deletion), a rebuild walks every `memories/*.md`, parses the YAML
//! frontmatter, and reinserts the `memories` + `memory_tags` + `memory_fts`
//! rows along with the graph edges harvested from the body.
//!
//! ## Conn-free by construction
//!
//! [`run`] never calls [`Ctx::conn`]: it opens its **own** standalone
//! connection on `comemory.db.rebuild.tmp` via
//! [`crate::store::connection::open`] and renames that file over the live
//! path. Over HTTP that matters twice — the job never contends for the
//! server's shared connection, and the server must swap its shared
//! connection afterwards (`serve::AppState::swap_conn`, spec §3 "Rebuild
//! connection swap") or keep reading the unlinked pre-rebuild inode.
//!
//! ## Atomic swap
//!
//! The new DB is built at `comemory.db.rebuild.tmp` so a crash or parse
//! error mid-rebuild leaves the original `comemory.db` intact. On success,
//! `fs::rename` replaces the live DB in one atomic filesystem operation.
//!
//! ## Code index + learning-state preservation
//!
//! Everything that exists only in SQLite — the code index (`code_symbols`,
//! `code_vec`, `code_fts`, `indexed_files`), the mined/earned code-graph
//! edges plus their `repo_marker` cursors, and the five learning-loop
//! tables — is copied from the old DB via `ATTACH DATABASE` before the
//! swap. Markdown cannot rebuild any of it, so dropping it would force a
//! full re-index and silently reset the feedback rerank priors. See
//! [`copy`] for the per-table details.
//!
//! Memory-side vectors are intentionally *not* repopulated: the v0.2
//! contract is BYO-vector, so re-embedding means re-running the caller's
//! embedder through `comemory save` / `ingest-code`. The lexical path
//! (`memory_fts`) is fully restored.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::api::Ctx;
use crate::memory::MemoryStore;
use crate::prelude::*;
use crate::store::{connection, memory_row};

/// `ATTACH`-based preservation copy of the code-index, code-graph, and
/// learning-loop tables from the pre-rebuild DB.
pub mod copy;

/// `comemory rebuild` / `POST /api/v1/rebuild` request. The command has no
/// flags today — it always rebuilds the entire memory layer of the SQLite
/// mirror from `memories/` while preserving the code index.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {}

/// Atomically rebuild the memory layer of `comemory.db` from markdown files,
/// preserving any existing code index tables.
///
/// 1. Build a fresh DB at `comemory.db.rebuild.tmp` (schema migrations run
///    normally on the temp path via `connection::open`).
/// 2. Walk `memories/` and insert every `memories` + `memory_tags` +
///    `memory_fts` + edges row into the temp DB.
/// 3. If the original `comemory.db` exists, `ATTACH` it and copy
///    `code_symbols`, `code_vec`, `code_fts`, and `indexed_files` rows into
///    the new DB so the code index survives the rebuild, plus the mined
///    code-graph edges + `repo_marker` cursors and the five learning tables
///    (`feedback`, `code_feedback`, `feedback_events`, `retrieval_log`,
///    `query_expansions`) so feedback counters and mined expansions do too.
/// 4. Close the temp connection then `fs::rename` it over the live path
///    (atomic on POSIX; on Windows this may fail if the DB is held open by
///    another process).
/// 5. Remove stale WAL/SHM sidecars from the original path so the next open
///    starts clean.
///
/// On any error the original DB is left untouched and the tmp file is
/// removed. Emits nothing on success — the HTTP job's `result` is `null`,
/// matching the CLI's silent success.
pub fn run(ctx: &mut Ctx<'_>, _req: Request) -> Result<()> {
    let paths = ctx.paths;
    paths.ensure_dirs()?;

    let db = paths.db_path();
    let tmp_path = {
        let mut p = db.clone().into_os_string();
        p.push(".rebuild.tmp");
        PathBuf::from(p)
    };

    // Best-effort cleanup of any leftover tmp + its WAL/SHM sidecars from a
    // previous crashed run. SQLite leaves `*-wal` / `*-shm` next to the main
    // file after a `PRAGMA journal_mode = WAL` open even on a clean close,
    // so the tmp path needs its sidecars removed alongside the main file or
    // the next rebuild reuses stale WALs.
    remove_db_and_sidecars(&tmp_path);

    match build_new_db(&db, &tmp_path, paths) {
        Ok(()) => swap_into_place(&tmp_path, &db),
        Err(e) => {
            // Remove the partial tmp + sidecars so the caller can retry cleanly.
            remove_db_and_sidecars(&tmp_path);
            Err(e)
        }
    }
}

/// Atomic swap: rename the freshly built tmp DB over the live path, then
/// remove stale WAL/SHM sidecars next to both (so the next open of
/// `comemory.db` starts clean and the just-renamed tmp connection's
/// `comemory.db.rebuild.tmp-wal` / `-shm` don't linger).
///
/// The rename result is matched rather than `?`-propagated so a failure
/// (cross-device, permission, the live DB held open exclusively on Windows)
/// still removes the tmp DB and lets the caller retry cleanly.
fn swap_into_place(tmp_path: &Path, db: &Path) -> Result<()> {
    match std::fs::rename(tmp_path, db) {
        Ok(()) => {
            remove_sidecars(db);
            remove_sidecars(tmp_path);
            Ok(())
        }
        Err(e) => {
            remove_db_and_sidecars(tmp_path);
            Err(Error::Io(e))
        }
    }
}

/// Remove `path` plus its SQLite WAL/SHM sidecars if present. Best-effort:
/// each removal is independent so a missing file does not abort the loop.
fn remove_db_and_sidecars(path: &Path) {
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    remove_sidecars(path);
}

/// Remove `path-wal` and `path-shm` if present. Best-effort.
fn remove_sidecars(path: &Path) {
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = path.to_path_buf().into_os_string();
        sidecar.push(suffix);
        let sidecar = PathBuf::from(sidecar);
        if sidecar.exists() {
            let _ = std::fs::remove_file(&sidecar);
        }
    }
}

/// Build the fresh DB at `tmp_path`, populate it from markdown, then copy the
/// code index tables from `old_db` if it exists. Extracted so the error path
/// in [`run`] can clean up the tmp file unconditionally.
fn build_new_db(old_db: &Path, tmp_path: &Path, paths: &crate::config::paths::Paths) -> Result<()> {
    let mut conn = connection::open(tmp_path)?;
    let tx = conn.transaction()?;

    let store = MemoryStore::new(paths.clone());
    for rec in store.list()? {
        let md_path = rec.path.to_string_lossy();
        memory_row::insert(
            &tx,
            &rec.frontmatter,
            &rec.body,
            rec.slug.as_str(),
            &md_path,
            &rec.frontmatter.tags,
        )?;
    }
    tx.commit()?;

    // Copy the code index + learning tables from the old DB if it exists.
    // A copy failure aborts the whole rebuild (tmp removed, live DB never
    // renamed over): learning state must not be silently dropped, and the
    // operator can retry once the source DB is readable again.
    if old_db.exists() {
        copy::copy_preserved_tables_from_old(&mut conn, old_db)?;
    }

    // Derived artifacts last: the replay supplies the memory→memory
    // relations and the copy above restores the mined code-graph edges, so
    // the graph is only whole now.
    crate::graph::derived::refresh_derived_best_effort(&mut conn);

    // Close the connection before rename by dropping it here.
    drop(conn);
    Ok(())
}
