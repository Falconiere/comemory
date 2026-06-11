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
//! ## Code index + learning-state preservation
//!
//! `code_symbols`, `code_vec`, `code_fts`, and `indexed_files` are copied
//! from the old DB into the new one via `ATTACH DATABASE` before the
//! connection is closed, so a rebuild triggered by a schema drift on the
//! memory side does not force a full re-index of the code corpus. The
//! learning-loop tables (`feedback`, `feedback_events`, `retrieval_log`,
//! `query_expansions`) are copied the same way: they exist only in SQLite
//! — there is no markdown to rebuild them from — and dropping them would
//! silently reset the Beta feedback rerank prior and erase mined
//! expansions, contradicting the documented never-expire contract.
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
///    the new DB so the code index survives the rebuild, plus the four
///    learning tables (`feedback`, `feedback_events`, `retrieval_log`,
///    `query_expansions`) so feedback counters and mined expansions do too.
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

    // Best-effort cleanup of any leftover tmp + its WAL/SHM sidecars from a
    // previous crashed run. SQLite leaves `*-wal` / `*-shm` next to the main
    // file after a `PRAGMA journal_mode = WAL` open even on a clean close,
    // so the tmp path needs its sidecars removed alongside the main file or
    // the next rebuild reuses stale WALs.
    remove_db_and_sidecars(&tmp_path);

    let result = build_new_db(&db, &tmp_path, &paths);

    match result {
        Ok(()) => {
            // Atomic swap: rename tmp over the live path. Capture the result
            // so we can clean up the tmp DB + its sidecars even on rename
            // failure — `?` would otherwise skip the cleanup blocks below
            // and leave the orphaned tmp file in the data dir.
            match std::fs::rename(&tmp_path, &db) {
                Ok(()) => {
                    // Remove stale WAL/SHM sidecars next to both the live DB
                    // (so the next open of `comemory.db` starts clean) and
                    // the tmp path (so the leftover
                    // `comemory.db.rebuild.tmp-wal` / `-shm` from the
                    // just-renamed tmp connection don't linger).
                    remove_sidecars(&db);
                    remove_sidecars(&tmp_path);
                    Ok(())
                }
                Err(e) => {
                    // Rename failed (e.g. cross-device, permission, the live
                    // DB held open exclusively on Windows). Remove the tmp DB
                    // + sidecars so the caller can retry cleanly.
                    remove_db_and_sidecars(&tmp_path);
                    Err(Error::Io(e))
                }
            }
        }
        Err(e) => {
            // Remove the partial tmp + sidecars so the caller can retry cleanly.
            remove_db_and_sidecars(&tmp_path);
            Err(e)
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

    // Copy the code index + learning tables from the old DB if it exists.
    // We do this outside the memory transaction so a copy failure doesn't
    // prevent the memory rebuild from landing; the worst outcome is a
    // missing code index that the operator can restore with
    // `comemory index-code`.
    if old_db.exists() {
        copy_preserved_tables_from_old(&mut conn, old_db)?;
    }

    // Close the connection before rename by dropping it here.
    drop(conn);
    Ok(())
}

/// Attach `old_db` as `old` and copy the four code-index tables plus the
/// four learning tables into the already-open `conn` (which points at the
/// tmp path). Uses INSERT-SELECT so no intermediate buffers are needed;
/// runs outside a transaction because vec0 virtual tables cannot
/// participate in user transactions.
///
/// The ATTACH path is bound via a parameter rather than interpolated into the
/// SQL so a data dir whose path contains a single quote (or other SQL
/// metacharacter) cannot break the statement.
///
/// Each source table is only copied if it actually exists on the attached
/// database: a v0.1 or otherwise legacy `comemory.db` may not have any of the
/// v2 code-index tables (and a pre-v5 one lacks `feedback_events` /
/// `query_expansions`), in which case the rebuild should still succeed
/// rather than failing with "no such table".
fn copy_preserved_tables_from_old(conn: &mut rusqlite::Connection, old_db: &Path) -> Result<()> {
    conn.execute(
        "ATTACH DATABASE ? AS old",
        rusqlite::params![old_db.to_string_lossy().as_ref()],
    )?;
    let copy_result = copy_code_tables_inner(conn).and_then(|()| copy_learning_tables_inner(conn));
    // Always DETACH so the connection is reusable even if the copy failed.
    let _ = conn.execute_batch("DETACH DATABASE old;");
    copy_result
}

/// Inner copy loop separated so the outer wrapper can guarantee `DETACH`
/// runs even on error.
///
/// Every copy lists its columns explicitly — `SELECT *` would break the
/// moment the attached DB predates a migration that widened a table,
/// because the old DB is attached raw and never migrated. A pre-v4
/// `code_symbols` lacks the `access_count` / `last_accessed` columns added
/// by migration 0004, so those two are sourced conditionally: carried over
/// when the old table already has them, otherwise synthesized with the
/// same defaults 0004's backfill applies (`0` / `indexed_at`).
fn copy_code_tables_inner(conn: &rusqlite::Connection) -> Result<()> {
    // Copy regular tables first, then the virtual tables (FTS5 + vec0).
    // code_symbols must come before code_vec/code_fts because the latter
    // reference code_symbols.id in their data streams.
    if old_table_exists(conn, "code_symbols")? {
        let (count_expr, last_expr) = if old_column_exists(conn, "code_symbols", "access_count")? {
            ("access_count", "COALESCE(last_accessed, indexed_at)")
        } else {
            ("0", "indexed_at")
        };
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.code_symbols(\
                 id, repo, path, blob_oid, symbol, kind, lang, line_start, line_end, \
                 snippet, simhash, indexed_at, access_count, last_accessed) \
             SELECT id, repo, path, blob_oid, symbol, kind, lang, line_start, line_end, \
                 snippet, simhash, indexed_at, {count_expr}, {last_expr} \
             FROM old.code_symbols;"
        ))?;
    }
    if old_table_exists(conn, "indexed_files")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.indexed_files(repo, path, blob_oid, indexed_at) \
             SELECT repo, path, blob_oid, indexed_at FROM old.indexed_files;",
        )?;
    }
    // FTS5 and vec0 virtual tables may not support `INSERT INTO … SELECT *`
    // from an attached DB in all sqlite-vec versions; copy each row
    // explicitly via named columns for safety. For code_fts we insert via
    // the FTS5 content table shape; vec0 rows are blobs tied to symbol_id.
    if old_table_exists(conn, "code_fts")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.code_fts(symbol_id, symbol, snippet, path_tokens) \
             SELECT symbol_id, symbol, snippet, path_tokens FROM old.code_fts;",
        )?;
    }
    if old_table_exists(conn, "code_vec")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.code_vec(symbol_id, embedding) \
             SELECT symbol_id, embedding FROM old.code_vec;",
        )?;
    }
    Ok(())
}

/// Inner copy loop for the learning-loop tables: `feedback` counters (v2),
/// `retrieval_log` telemetry (v3), `feedback_events` provenance and mined
/// `query_expansions` (both v5). These rows exist only in SQLite — there is
/// no markdown source to rebuild them from — so a rebuild that dropped them
/// would silently reset the Beta feedback rerank prior to neutral and erase
/// mined expansions, contradicting the documented never-expire contract.
///
/// Same schema-evolution guards as [`copy_code_tables_inner`]: each table is
/// only copied when it exists on the attached DB, and
/// `retrieval_log.duration_ms` (added in v5) is probed via
/// [`old_column_exists`] and defaulted to NULL when the source predates it.
fn copy_learning_tables_inner(conn: &rusqlite::Connection) -> Result<()> {
    if old_table_exists(conn, "feedback")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.feedback(\
                 memory_id, used_count, irrelevant_count, last_used) \
             SELECT memory_id, used_count, irrelevant_count, last_used \
             FROM old.feedback;",
        )?;
    }
    if old_table_exists(conn, "retrieval_log")? {
        let duration_expr = if old_column_exists(conn, "retrieval_log", "duration_ms")? {
            "duration_ms"
        } else {
            "NULL"
        };
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.retrieval_log(\
                 query_id, query, returned_ids, at, duration_ms) \
             SELECT query_id, query, returned_ids, at, {duration_expr} \
             FROM old.retrieval_log;"
        ))?;
    }
    if old_table_exists(conn, "feedback_events")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.feedback_events(\
                 id, query_id, memory_id, verdict, at) \
             SELECT id, query_id, memory_id, verdict, at \
             FROM old.feedback_events;",
        )?;
    }
    if old_table_exists(conn, "query_expansions")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.query_expansions(\
                 term, expansion, support, last_mined) \
             SELECT term, expansion, support, last_mined \
             FROM old.query_expansions;",
        )?;
    }
    Ok(())
}

/// True when `name` exists as a table (regular or virtual) on the attached
/// `old` database. Lets the copy loop skip tables that predate v0.2.
fn old_table_exists(conn: &rusqlite::Connection, name: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM old.sqlite_master WHERE type = 'table' AND name = ?1",
        rusqlite::params![name],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// True when `column` exists on `table` in the attached `old` database.
/// Lets [`copy_code_tables_inner`] and [`copy_learning_tables_inner`] adapt
/// their SELECT lists to the attached DB's schema version instead of
/// assuming the current one.
fn old_column_exists(conn: &rusqlite::Connection, table: &str, column: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM pragma_table_info(?1, 'old') WHERE name = ?2",
        rusqlite::params![table, column],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}
