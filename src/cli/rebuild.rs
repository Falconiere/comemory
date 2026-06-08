//! `comemory rebuild` — atomically replace the SQLite mirror, preserving the
//! code index, by rebuilding from the on-disk markdown files.
//!
//! Markdown remains the source of truth in v0.2; `comemory.db` is a
//! rebuildable derived cache. When the DB drifts (schema change, corruption,
//! manual deletion), `comemory rebuild` walks every `memories/*.md`, parses
//! the YAML frontmatter, and reinserts the `memories` + `memory_tags` +
//! `memory_fts` rows along with the graph edges harvested from the body.
//!
//! ## Atomic swap
//!
//! The new DB is built at `comemory.db.rebuild.tmp` so a crash or parse
//! error mid-rebuild leaves the original `comemory.db` intact. On success,
//! `fs::rename` replaces the live DB in one atomic filesystem operation.
//!
//! ## Code index preservation
//!
//! `code_symbols`, `code_vec`, `code_fts`, and `indexed_files` are copied
//! from the old DB into the new one via `ATTACH DATABASE` before the
//! connection is closed, so a rebuild triggered by a schema drift on the
//! memory side does not force a full re-index of the code corpus.
//!
//! Vectors are intentionally *not* repopulated here for the memory side:
//! the v0.2 contract is BYO-vector, so re-embedding requires running the
//! caller's embedder and piping through `comemory save` or `ingest-code`.
//! The lexical path (`memory_fts`) is fully restored, which is enough to
//! answer the lexical branch of the router.

use std::path::{Path, PathBuf};

use clap::Args as ClapArgs;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::prelude::*;
use crate::store::{connection, memory_row};

/// Arguments to `comemory rebuild`. Currently no flags — the command always
/// rebuilds the entire memory layer of the SQLite mirror from `memories/`
/// while preserving the code index. Wrapped in a struct so future opt-in
/// flags (e.g. `--keep-stats`, `--dry-run`) can land without breaking the
/// dispatcher signature.
#[derive(ClapArgs, Debug)]
pub struct Args;

/// Atomically rebuild the memory layer of `comemory.db` from markdown files,
/// preserving any existing code index tables.
///
/// 1. Build a fresh DB at `comemory.db.rebuild.tmp` (schema migrations run
///    normally on the temp path via `connection::open`).
/// 2. Walk `memories/` and insert every `memories` + `memory_tags` +
///    `memory_fts` + edges row into the temp DB.
/// 3. If the original `comemory.db` exists, `ATTACH` it and copy
///    `code_symbols`, `code_vec`, `code_fts`, and `indexed_files` rows into
///    the new DB so the code index survives the rebuild.
/// 4. Close the temp connection then `fs::rename` it over the live path
///    (atomic on POSIX; on Windows this may fail if the DB is held open by
///    another process).
/// 5. Remove stale WAL/SHM sidecars from the original path so the next open
///    starts clean.
///
/// On any error the original DB is left untouched and the tmp file is removed.
pub async fn run(_args: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;

    let db = paths.db_path();
    let tmp_path = {
        let mut p = db.clone().into_os_string();
        p.push(".rebuild.tmp");
        PathBuf::from(p)
    };

    // Best-effort cleanup of any leftover tmp from a previous crashed run.
    if tmp_path.exists() {
        let _ = std::fs::remove_file(&tmp_path);
    }

    let result = build_new_db(&db, &tmp_path, &paths);

    match result {
        Ok(()) => {
            // Atomic swap: rename tmp over the live path.
            std::fs::rename(&tmp_path, &db).map_err(Error::Io)?;
            // Remove stale WAL/SHM sidecars so the next open starts clean.
            for suffix in ["-wal", "-shm"] {
                let mut sidecar = db.clone().into_os_string();
                sidecar.push(suffix);
                let sidecar = PathBuf::from(sidecar);
                if sidecar.exists() {
                    let _ = std::fs::remove_file(&sidecar);
                }
            }
            Ok(())
        }
        Err(e) => {
            // Remove the partial tmp so the caller can retry cleanly.
            let _ = std::fs::remove_file(&tmp_path);
            Err(e)
        }
    }
}

/// Build the fresh DB at `tmp_path`, populate it from markdown, then copy the
/// code index tables from `old_db` if it exists. Extracted so the error path
/// in `run` can clean up the tmp file unconditionally.
fn build_new_db(old_db: &Path, tmp_path: &Path, paths: &Paths) -> Result<()> {
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

    // Copy code index from the old DB if it exists. We do this outside the
    // memory transaction so a code-copy failure doesn't prevent the memory
    // rebuild from landing; the worst outcome is a missing code index that
    // the operator can restore with `comemory index-code`.
    if old_db.exists() {
        copy_code_tables_from_old(&mut conn, old_db)?;
    }

    // Close the connection before rename by dropping it here.
    drop(conn);
    Ok(())
}

/// Attach `old_db` as `old` and copy the four code-index tables into the
/// already-open `conn` (which points at the tmp path). Uses INSERT-SELECT so
/// no intermediate buffers are needed; runs outside a transaction because
/// vec0 virtual tables cannot participate in user transactions.
fn copy_code_tables_from_old(conn: &mut rusqlite::Connection, old_db: &Path) -> Result<()> {
    let old_db_str = old_db.to_string_lossy();
    conn.execute_batch(&format!("ATTACH DATABASE '{old_db_str}' AS old;"))?;
    // Copy regular tables first, then the virtual tables (FTS5 + vec0).
    // code_symbols must come before code_vec/code_fts because the latter
    // reference code_symbols.id in their data streams.
    for table in ["code_symbols", "indexed_files"] {
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.{table} SELECT * FROM old.{table};"
        ))?;
    }
    // FTS5 and vec0 virtual tables may not support `INSERT INTO … SELECT *`
    // from an attached DB in all sqlite-vec versions; copy each row
    // explicitly via named columns for safety. For code_fts we insert via
    // the FTS5 content table shape; vec0 rows are blobs tied to symbol_id.
    conn.execute_batch(
        "INSERT OR IGNORE INTO main.code_fts(symbol_id, symbol, snippet, path_tokens) \
         SELECT symbol_id, symbol, snippet, path_tokens FROM old.code_fts;",
    )?;
    conn.execute_batch(
        "INSERT OR IGNORE INTO main.code_vec(symbol_id, embedding) \
         SELECT symbol_id, embedding FROM old.code_vec;",
    )?;
    conn.execute_batch("DETACH DATABASE old;")?;
    Ok(())
}
