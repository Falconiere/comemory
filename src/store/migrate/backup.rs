//! Pre-migration / pre-rebuild snapshots of `comemory.db`.
//!
//! `VACUUM INTO` is preferred over `fs::copy` because a bare file copy would
//! omit committed-but-not-yet-checkpointed frames still living in
//! `comemory.db-wal` — `VACUUM INTO` reads through SQLite itself, so the
//! snapshot always reflects every committed row regardless of checkpoint
//! state. It cannot run inside a transaction (SQLite will not serialize it
//! on its own — callers take [`crate::source::lock::FileLock`] around it)
//! and it errors rather than overwriting an existing destination, so
//! [`snapshot`] validates and clears a stale destination first.
//!
//! There is deliberately no free-space precheck: `std` exposes no such API
//! and `libc`/`nix` are dev-dependencies only. `SQLITE_FULL` surfaces from
//! `VACUUM INTO` like any other SQLite error and is handled by the caller's
//! class-dependent warn-or-refuse branch.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::Connection;

use crate::prelude::*;

/// How many snapshots matching a given `prefix` [`prune`] keeps.
const KEEP: usize = 2;

/// `VACUUM INTO` a snapshot of `conn`'s database through an already-open
/// connection. `dest` is bound as a query parameter, never
/// string-interpolated, so a data directory whose path contains a quote (or
/// other SQL metacharacter) cannot break the statement — the same reasoning
/// as `api::rebuild::copy`'s `ATTACH`.
///
/// If `dest` already holds a file that passes `PRAGMA quick_check`, the
/// snapshot is skipped as an idempotent no-op (`VACUUM INTO` errors rather
/// than overwriting). A `dest` that exists but fails that check — e.g. a
/// truncated file left behind by a killed process — is removed and
/// recreated instead of being trusted forever.
pub(crate) fn snapshot(conn: &Connection, dest: &Path) -> Result<()> {
    if reuse_existing(dest)? {
        return Ok(());
    }
    conn.execute(
        "VACUUM INTO ?1",
        rusqlite::params![dest.to_string_lossy().as_ref()],
    )?;
    Ok(())
}

/// Snapshot the database file at `src`, which nothing currently holds open.
/// Opens a plain `rusqlite::Connection::open` — deliberately NOT
/// [`crate::store::connection::open`], which would run preflight and the
/// whole migration chain against a database that is about to be replaced.
/// Used by `comemory rebuild`, where the pre-swap database's connection has
/// already been dropped by the time the swap runs.
pub(crate) fn snapshot_path(src: &Path, dest: &Path) -> Result<()> {
    let conn = Connection::open(src)?;
    snapshot(&conn, dest)
}

/// True when `dest` already exists and is a valid, reusable snapshot — the
/// idempotent-skip case [`snapshot`] returns early on. A `dest` that exists
/// but fails validation is removed so the caller's `VACUUM INTO` can
/// recreate it; a `dest` that does not exist at all is left alone for
/// `VACUUM INTO` to create.
fn reuse_existing(dest: &Path) -> Result<bool> {
    if !dest.exists() {
        return Ok(false);
    }
    if is_valid_sqlite(dest) {
        return Ok(true);
    }
    std::fs::remove_file(dest)?;
    Ok(false)
}

/// True when the SQLite file at `path` passes `PRAGMA quick_check`. Opens
/// its own connection rather than reusing a caller's, since this validates
/// the *destination* — a different file from whatever connection the
/// caller is snapshotting.
///
/// Registers the `identifier` FTS5 tokenizer on this connection first: any
/// real (v4+) comemory database carries `tokenize = 'identifier'` columns
/// (`memory_fts`, `code_fts`, ...), and `quick_check` touches every table
/// including FTS5 shadow tables — without registration it fails outright
/// with "error in tokenizer constructor" on precisely the snapshots this
/// function exists to validate, which would silently defeat the
/// idempotent-reuse path for every real database.
fn is_valid_sqlite(path: &Path) -> bool {
    let Ok(conn) = Connection::open(path) else {
        return false;
    };
    if crate::store::tokenizer::ffi::register(&conn).is_err() {
        return false;
    }
    conn.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
        .is_ok_and(|result| result == "ok")
}

/// Keep the newest [`KEEP`] files directly under `data_dir` whose name
/// starts with `prefix`, deleting the rest. Prefix-scoped so the migration
/// budget (`comemory.db.pre-v*`) and the rebuild budget
/// (`comemory.db.pre-rebuild`) cannot evict each other. Run before a
/// snapshot, since `VACUUM INTO` errors rather than overwriting an existing
/// destination.
pub(crate) fn prune(data_dir: &Path, prefix: &str) -> Result<()> {
    let mut matches = matching_files(data_dir, prefix)?;
    matches.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, path) in matches.into_iter().skip(KEEP) {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// `(modified_time, path)` for every regular file directly under
/// `data_dir` whose name starts with `prefix`. A missing `data_dir` yields
/// an empty list rather than an error — nothing has been snapshotted yet.
fn matching_files(data_dir: &Path, prefix: &str) -> Result<Vec<(SystemTime, PathBuf)>> {
    let entries = match std::fs::read_dir(data_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry?;
        let is_match = entry
            .file_name()
            .to_str()
            .is_some_and(|s| s.starts_with(prefix));
        if !is_match {
            continue;
        }
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        out.push((meta.modified()?, entry.path()));
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests/backup.rs"]
mod tests;
