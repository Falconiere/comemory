//! `api::stats::{Request, run}` — the shared middle of `comemory stats` /
//! `GET /api/v1/stats`: corpus counters and on-disk size in one round trip.
//!
//! Every one of these numbers was previously unreachable without opening
//! `comemory.db` by hand, which is why the console's Overview tiles
//! (memories, code symbols, graph edges, database size) and its nav badges
//! had no producer.
//!
//! **Must-not-create-the-db invariant** (the same rule `api::gc` keeps): a
//! read command must not create and migrate a database as a side effect of
//! being asked how big one is. On a data dir with no `comemory.db`, `run`
//! never calls [`Ctx::conn`] — it reports the markdown count it can get
//! from the filesystem, zeros for every SQL-backed counter, and
//! `schema_version: "unknown"`.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::config::Paths;
use crate::prelude::*;

/// `comemory stats` / `GET /api/v1/stats` request.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Scope the per-repo counters (`memories`, `trashed`, `code_symbols`,
    /// `documents`) to one repo label. `edges`, `db_bytes`, `repos`, and
    /// `markdown_files` stay global — an edge is a cross-kind row, and a
    /// database file has one size no matter who asks.
    #[serde(default)]
    pub repo: Option<String>,
}

/// Corpus counters as emitted under `--json` and in the `/api/v1/stats`
/// `data` field.
#[derive(Serialize, Debug)]
pub struct Response {
    /// Live memory rows (`deleted_at IS NULL`).
    pub memories: u64,
    /// Soft-deleted memory rows still in the mirror.
    pub trashed: u64,
    /// `*.md` files directly under `memories/` (the `.trash/` subdirectory
    /// is not counted — those are the `trashed` rows' files).
    pub markdown_files: u64,
    /// Rows in `code_symbols`, including cAST child chunks.
    pub code_symbols: u64,
    /// Rows in `documents`.
    pub documents: u64,
    /// Rows in `edges`, every relation kind together.
    pub edges: u64,
    /// `page_count * page_size`. Deliberately not the file's length on
    /// disk: the connection runs in WAL mode, so pages that have not been
    /// checkpointed yet live in `comemory.db-wal` and the two legitimately
    /// disagree.
    pub db_bytes: u64,
    /// Distinct repos in `repo_marker` — the code index's inventory size.
    pub repos: u64,
    /// Applied schema version, or `"unknown"` when no database exists yet.
    pub schema_version: String,
}

/// Collect the counters. See the module doc for why a missing database is
/// reported rather than created.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    let markdown_files = count_markdown(ctx.paths);
    if !ctx.paths.db_path().exists() {
        return Ok(Response {
            memories: 0,
            trashed: 0,
            markdown_files,
            code_symbols: 0,
            documents: 0,
            edges: 0,
            db_bytes: 0,
            repos: 0,
            schema_version: "unknown".into(),
        });
    }
    let repo = req.repo.as_deref();
    let conn = ctx.conn()?;
    Ok(Response {
        memories: scoped_count(conn, "memories", "deleted_at IS NULL", repo)?,
        trashed: scoped_count(conn, "memories", "deleted_at IS NOT NULL", repo)?,
        markdown_files,
        code_symbols: scoped_count(conn, "code_symbols", "1 = 1", repo)?,
        documents: scoped_count(conn, "documents", "1 = 1", repo)?,
        edges: count(conn, "SELECT COUNT(*) FROM edges")?,
        db_bytes: db_bytes(conn)?,
        repos: count(conn, "SELECT COUNT(*) FROM repo_marker")?,
        schema_version: conn.query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |r| r.get(0),
        )?,
    })
}

/// `*.md` files directly under `memories/`. A missing directory counts as
/// zero rather than failing: `stats` on a fresh data dir is a legitimate
/// question, not an error.
fn count_markdown(paths: &Paths) -> u64 {
    let Ok(rd) = std::fs::read_dir(paths.memories_dir()) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .count() as u64
}

/// `COUNT(*)` over `table` under `predicate`, narrowed to `repo` when one
/// was requested. Every table this is called with carries a nullable `repo`
/// column, so the filter is one shared `AND repo = ?` rather than three
/// hand-written queries that could drift.
fn scoped_count(
    conn: &Connection,
    table: &str,
    predicate: &str,
    repo: Option<&str>,
) -> Result<u64> {
    if let Some(repo) = repo {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE {predicate} AND repo = ?1");
        Ok(conn.query_row(&sql, [repo], |r| r.get::<_, i64>(0))? as u64)
    } else {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE {predicate}");
        count(conn, &sql)
    }
}

/// Run a parameterless `COUNT(*)` query.
fn count(conn: &Connection, sql: &str) -> Result<u64> {
    Ok(conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64)
}

/// `page_count * page_size` — the logical size of the database, see
/// [`Response::db_bytes`] for why this is not the file length.
fn db_bytes(conn: &Connection) -> Result<u64> {
    let pages: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    let size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    Ok((pages.max(0) as u64).saturating_mul(size.max(0) as u64))
}

#[cfg(test)]
#[path = "tests/stats.rs"]
mod tests;
