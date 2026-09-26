//! `maintenance::rebuild::{Request, run}` — the shared middle of `comemory rebuild`
//! / `POST /api/v1/rebuild` (job): replace the SQLite mirror's content in
//! place, preserving the code index, by rebuilding from the on-disk markdown
//! files. Moved out of `cli::rebuild::run` (Binding Rule 1).
//!
//! Markdown remains the source of truth in v0.2; `comemory.db` is a
//! rebuildable derived cache. When the DB drifts (schema change, corruption,
//! manual deletion), a rebuild walks every `memories/*.md`, parses the YAML
//! frontmatter, and reinserts the `memories` + `memory_tags` + `memory_fts`
//! rows along with the graph edges harvested from the body.
//!
//! ## Staged, then replaced in place
//!
//! [`stage`] builds the new DB at `comemory.db.rebuild.tmp` on its own
//! connection, so a crash or parse error mid-rebuild leaves `comemory.db`
//! intact. [`Staged::replace_into`] then copies it INTO a connection already
//! open on the live file through SQLite's online backup API
//! ([`crate::store::replace_in_place`], #256) — one write transaction, no
//! rename. Every connection open on `comemory.db` (the server's shared one,
//! an MCP session's) reads the rebuilt content on its next query, and no
//! `-wal`/`-shm` sidecar is removed from under it. [`run`] replaces through a
//! connection of its own; the HTTP job replaces through the server's shared
//! connection, so it needs no reopen and no swap.
//!
//! ## Writers paused
//!
//! [`stage`] takes `memory-save.lock` before reading anything and the
//! [`Staged`] value holds it until the replace is done, so no markdown writer
//! can land between the markdown walk and the copy. It waits `[sync]
//! pause_wait` for a current holder, then fails [`Error::Busy`] having
//! changed nothing — as does a save waiting on a rebuild.
//!
//! ## Code index + learning-state preservation
//!
//! Everything that exists only in SQLite — the code index (`code_symbols`,
//! `code_vec`, `code_fts`, `indexed_files`), the mined/earned code-graph
//! edges plus their `repo_marker` cursors, the five learning-loop tables,
//! and the document-domain tables (`source_files`, `documents`,
//! `document_chunks`, `document_fts`) — is copied from the old DB via
//! `ATTACH DATABASE` before the replace by [`crate::store::rebuild_copy`]
//! and its siblings. Markdown cannot rebuild any of it, so dropping it would
//! force a full re-index and silently reset the feedback rerank priors. See
//! [`copy`] for the live-table allowlist. The fifth v13 table,
//! `source_roots`, is reconciled fresh from `sources.toml`
//! (`source::mirror::reconcile`) rather than copied — see [`build_new_db`].
//!
//! Memory-side vectors are intentionally *not* repopulated: the v0.2
//! contract is BYO-vector, so re-embedding means re-running the caller's
//! embedder through `comemory save` / `ingest-code`. The lexical path
//! (`memory_fts`) is fully restored.
//!
//! ## Pre-replace snapshot
//!
//! Before [`Staged::replace_into`] overwrites the live content,
//! [`snapshot_before_swap`] `VACUUM INTO`s it to
//! [`crate::config::paths::Paths::rebuild_backup`] — the exact operation
//! that once discarded a full release's document index with no recovery
//! path at all.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::Config;
use crate::config::paths::Paths;
use crate::domains::documents::source::mirror;
use crate::domains::documents::source::registry::Registry;
use crate::domains::memories::{MemoryStore, SaveGuard, save_lock};
use crate::domains::sync::replica::bootstrap;
use crate::prelude::*;
use crate::store::connection;
use crate::store::migrate::backup;
use crate::store::{Connection, replace_in_place, schema_meta, seed_scan};
use crate::utilities::context::Ctx;

/// The live-table allowlist pair plus the thin delegate into
/// `crate::store::rebuild_copy`, which owns the actual `ATTACH`-based
/// preservation copy of the code-index, code-graph, learning-loop, and
/// document-domain tables from the pre-rebuild DB.
pub mod copy;

/// `comemory rebuild` / `POST /api/v1/rebuild` request. The command has no
/// flags today — it always rebuilds the entire memory layer of the SQLite
/// mirror from `memories/` while preserving the code index.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {}

/// Rebuild the memory layer of `comemory.db` from markdown files, preserving
/// any existing code index tables: [`stage`] the new DB, then replace the
/// live content in place through a connection this call opens on it.
///
/// Never calls [`Ctx::conn`]: the destination is a plain connection with no
/// migration, since its content is about to be replaced wholesale. On any
/// error the live DB is left untouched and the tmp file is removed. Emits
/// nothing on success — the HTTP job's `result` is `null`, matching the
/// CLI's silent success.
pub fn run(ctx: &mut Ctx<'_>, _req: Request) -> Result<()> {
    let pause_wait = ctx.cfg.sync.pause_wait_duration()?;
    let staged = stage(ctx.paths, ctx.cfg)?;
    let mut live = replace_in_place::open_destination(&ctx.paths.db_path(), pause_wait)?;
    staged.replace_into(&mut live)
}

/// A rebuilt database staged beside the live one, holding `memory-save.lock`
/// until [`Staged::replace_into`] installs it. Dropping it — installed or
/// not — removes the tmp file and its sidecars, then releases the lock.
pub struct Staged {
    tmp_path: PathBuf,
    _guard: SaveGuard,
}

impl Staged {
    /// Replace `live`'s content with the staged database in place, through a
    /// connection already open on `comemory.db` — the server passes its
    /// shared one, so its next request reads the rebuilt content.
    ///
    /// # Errors
    /// [`Error::Busy`] when another connection holds the write lock past
    /// `live`'s busy timeout; the live content is then unchanged.
    pub fn replace_into(self, live: &mut Connection) -> Result<()> {
        replace_in_place::replace_in_place(live, &self.tmp_path)
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        remove_db_and_sidecars(&self.tmp_path);
    }
}

/// Pause markdown writers, build the new DB at `comemory.db.rebuild.tmp`
/// from markdown plus the preserved tables, and snapshot the still-live
/// `comemory.db` to [`crate::config::paths::Paths::rebuild_backup`].
///
/// # Errors
/// [`Error::Busy`] when `memory-save.lock` is still held after `[sync]
/// pause_wait` — before anything is read or written. Any build or snapshot
/// failure removes the tmp file and leaves the live DB untouched.
pub fn stage(paths: &Paths, cfg: &Config) -> Result<Staged> {
    paths.ensure_dirs()?;
    let guard = save_lock::acquire_within(paths, cfg.sync.pause_wait_duration()?)?;
    let db = paths.db_path();
    let tmp_path = {
        let mut p = db.clone().into_os_string();
        p.push(".rebuild.tmp");
        PathBuf::from(p)
    };
    // Clear a tmp (and the `-wal`/`-shm` SQLite leaves beside it) from a
    // previous crashed run before reusing the path.
    remove_db_and_sidecars(&tmp_path);
    // From here every early return drops `staged`, which removes the tmp.
    let staged = Staged {
        tmp_path,
        _guard: guard,
    };
    build_new_db(&db, &staged.tmp_path, paths)?;
    if db.exists() {
        snapshot_before_swap(&db, paths)?;
    }
    Ok(staged)
}

/// Snapshot the live `comemory.db` at `db` to
/// [`crate::config::paths::Paths::rebuild_backup`] before
/// [`Staged::replace_into`] overwrites its content — the very operation that
/// once silently discarded a full release's document index. Uses
/// [`backup::snapshot_path`], which opens a plain connection of its own:
/// opening `db` via `connection::open` would run preflight and the whole
/// migration chain on a database whose content is about to be replaced.
///
/// `rebuild_backup()` is a single fixed path, unlike the migration backups'
/// per-version names, so a second rebuild would otherwise either fail
/// outright (`VACUUM INTO` errors rather than overwriting) or, if the prior
/// `.bak` were unlinked up front, leave the operator with NEITHER the prior
/// backup nor a new one when the new snapshot then failed for a
/// non-blocking reason (e.g. `SQLITE_FULL`). [`snapshot_before_swap_inner`]
/// avoids that by `VACUUM INTO`-ing to a sibling staging path first and
/// `fs::rename`-ing it over `dest` only once that has actually succeeded, so
/// a prior good `.bak` survives a failed attempt at a new one. Any failure —
/// the staging snapshot or the final rename — is wrapped with the
/// destination path, since the bare underlying error (an `io::Error` or a
/// generic SQLite failure) does not otherwise name what it was trying to
/// write.
fn snapshot_before_swap(db: &Path, paths: &Paths) -> Result<()> {
    let dest = paths.rebuild_backup();
    snapshot_before_swap_inner(db, &dest).map_err(|e| {
        Error::Other(format!(
            "pre-rebuild snapshot to {} failed: {e}",
            dest.display()
        ))
    })
}

/// The body of [`snapshot_before_swap`], separated so the caller can wrap
/// every error path (the staging snapshot or the final rename) with one
/// destination-naming context. `VACUUM INTO`s to a sibling `.tmp` staging
/// path, then renames it over `dest` only on success — a failed `VACUUM
/// INTO` never touches `dest` itself, so a prior good backup there is never
/// destroyed by a failed attempt at a new one.
pub(crate) fn snapshot_before_swap_inner(db: &Path, dest: &Path) -> Result<()> {
    let staging = staging_dest(dest);
    if staging.exists() {
        // Best-effort cleanup of a stale staging file left by a previous
        // attempt that crashed between the VACUUM INTO succeeding and the
        // rename below.
        std::fs::remove_file(&staging)?;
    }
    backup::snapshot_path(db, &staging)?;
    std::fs::rename(&staging, dest)?;
    Ok(())
}

/// Sibling staging path `dest` is snapshotted into first, so a failed
/// `VACUUM INTO` never touches `dest` (an existing, still-good `.bak`)
/// before the new snapshot is known to have succeeded.
fn staging_dest(dest: &Path) -> PathBuf {
    let mut p = dest.as_os_str().to_os_string();
    p.push(".tmp");
    PathBuf::from(p)
}

/// Remove the tmp DB at `path` plus its SQLite `-wal`/`-shm` sidecars. Each
/// removal is independent; a missing file is the normal case, and any other
/// failure is logged rather than returned — this runs from [`Staged`]'s
/// `Drop`, where there is no caller to hand it to, and the next rebuild
/// clears the path again before reusing it.
pub(crate) fn remove_db_and_sidecars(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut target = path.as_os_str().to_os_string();
        target.push(suffix);
        let target = PathBuf::from(target);
        if let Err(e) = std::fs::remove_file(&target)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %target.display(), error = %e, "could not remove a rebuild tmp file");
        }
    }
}

/// Build the fresh DB at `tmp_path`, populate it from markdown, reconcile
/// `source_roots` from `sources.toml`, then copy the code index +
/// document-domain tables from `old_db` if it exists. [`stage`] owns the
/// cleanup: an error here drops its [`Staged`], which removes the tmp.
fn build_new_db(old_db: &Path, tmp_path: &Path, paths: &Paths) -> Result<()> {
    let mut conn = connection::open(tmp_path)?;
    let tx = conn.transaction()?;

    let store = MemoryStore::new(paths.clone());
    for rec in store.list()? {
        let md_path = rec.path.to_string_lossy();
        crate::domains::memories::mirror::insert_row(
            &tx,
            &rec.frontmatter,
            &rec.body,
            rec.slug.as_str(),
            &md_path,
            &rec.frontmatter.tags,
        )?;
    }
    tx.commit()?;

    // Reconstruct `source_roots` fresh from `sources.toml` rather than
    // copying it from the old DB: it is the durable source of truth
    // (`RECONSTRUCTABLE_TABLES`), and its rows must exist before the copy
    // below, since `source_files.source_id` is a foreign key under
    // `PRAGMA foreign_keys=ON`.
    let registry = Registry::new(paths.clone());
    mirror::reconcile(&conn, &registry.load()?)?;

    // Copy the code index + learning + document-domain tables from the old
    // DB if it exists. A copy failure aborts the whole rebuild (tmp
    // removed, live DB never replaced): learning state must not be
    // silently dropped, and the operator can retry once the source DB is
    // readable again.
    if old_db.exists() {
        copy::copy_preserved_tables_from_old(&mut conn, old_db)?;
    }
    reset_seeding_if_unjournalled(&conn)?;

    // Derived artifacts last: the replay supplies the memory→memory
    // relations and the copy above restores the mined code-graph edges, so
    // the graph is only whole now.
    //
    // Its outcome is bound and dropped rather than reported: `run` emits
    // nothing on success by contract (the CLI is silent, the job's
    // `result` is `null`), and a rebuild whose rows are all in place has
    // not failed because a derived index needs another pass. The failure
    // is logged by the refresh itself, and the next save, delete or
    // index-code run rebuilds the index. `gc` and `delete` DO report it —
    // they have a response with somewhere to put it.
    let _stale = crate::domains::graph::derived::refresh_derived_best_effort(&mut conn);

    // Close the connection so the replace reads a checkpointed file.
    drop(conn);
    Ok(())
}

/// Reset the memory bootstrap scan to the beginning when the copy left a
/// live memory with no `replica_revision` row (#256) — a `schema_meta`
/// cursor the copy could not carry (a database this old, or a markdown file
/// placed in `memories/` by hand) would otherwise report `complete` and
/// withhold no capability, even though the journal is not actually whole.
fn reset_seeding_if_unjournalled(conn: &Connection) -> Result<()> {
    if seed_scan::unjournalled_live_memory_count(conn)? == 0 {
        return Ok(());
    }
    schema_meta::upsert(conn, bootstrap::STATE_KEY, "pending")?;
    schema_meta::upsert(conn, bootstrap::THROUGH_KEY, "")?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/rebuild.rs"]
mod tests;
#[cfg(test)]
#[path = "tests/rebuild_2.rs"]
mod tests_2;
