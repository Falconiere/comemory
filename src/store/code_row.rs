//! Shared SQLite-mirror writer for a single code-symbol row.
//!
//! Both `cli::index_code` (tree-sitter walk → INSERT) and `cli::ingest_code`
//! (pre-embedded JSONL → INSERT) materialise rows into the same v0.2
//! `code_symbols` table with byte-identical column lists. Centralising the
//! `INSERT ... RETURNING id` SQL here means the two paths cannot drift on
//! column order, timestamp expression, or `RETURNING` clause.
//!
//! The caller decides whether to also write a `code_vec` row — vectors are
//! BYO so `index_code` skips them while `ingest_code` always supplies one.
//!
//! [`purge_file_symbols`] and [`upsert_indexed_file`] are shared with
//! `ingest_code` so re-ingesting a previously-ingested `(repo, path)` does not
//! collide on the `UNIQUE (repo, path, symbol, line_start)` constraint and so
//! the `indexed_files` cursor reflects the most-recent ingest as well.

use rusqlite::Connection;
use time::OffsetDateTime;

use crate::prelude::*;
use crate::store::memory_row;

/// On-disk format version of the extracted rows for one repo. Stamped
/// per-repo in `schema_meta` under `code_format:<repo>`; when the stamp
/// disagrees (e.g. rows indexed before cAST chunking landed), every
/// `indexed_files` cursor for the repo is dropped so the next walk
/// re-extracts all files under the current format. Version "2" = cAST
/// chunk children with `parent_id` (the value the v6 migration writes
/// to the global `code_format_version` key).
///
/// Lives here (not in `cli::index_code`) because BOTH writers must stamp:
/// an `ingest-code` run that skipped the stamp would leave
/// [`ensure_repo_format`] seeing a missing/stale stamp on the next
/// `index-code`, which drops the repo's `indexed_files` cursors and the
/// full re-walk purges every freshly-ingested `code_vec` embedding.
pub(crate) const CODE_FORMAT_VERSION: &str = "2";

/// `schema_meta` key prefix of the per-repo code-format stamps
/// (`code_format:<repo>`). Shared with `cli::rebuild`, which copies the
/// stamps by prefix-matching `schema_meta` keys — interpolating this
/// const (and its `.len()`) into that SQL keeps the two sides from
/// drifting on the prefix or its length. The global
/// `code_format_version` key does NOT carry the trailing colon and never
/// matches.
pub(crate) const CODE_FORMAT_KEY_PREFIX: &str = "code_format:";

/// `schema_meta` key carrying the per-repo code format stamp.
fn repo_format_key(repo: &str) -> String {
    format!("{CODE_FORMAT_KEY_PREFIX}{repo}")
}

/// Drop every `indexed_files` cursor for `repo` when its per-repo format
/// stamp (`schema_meta` key `code_format:<repo>`) is missing or differs
/// from [`CODE_FORMAT_VERSION`], forcing the walk/stream that follows to
/// re-extract every file. [`purge_file_symbols`] then replaces the stale
/// per-file rows as the walk proceeds. Shared by `cli::index_code` and
/// `cli::ingest_code` so the two writers cannot drift on the gate.
pub(crate) fn ensure_repo_format(conn: &Connection, repo: &str) -> Result<()> {
    let stamped: Option<String> = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = ?1",
            [repo_format_key(repo)],
            |r| r.get(0),
        )
        .ok();
    if stamped.as_deref() != Some(CODE_FORMAT_VERSION) {
        conn.execute("DELETE FROM indexed_files WHERE repo = ?1", [repo])?;
    }
    Ok(())
}

/// Upsert the per-repo format stamp after a successful walk/stream so the
/// next run skips the [`ensure_repo_format`] cursor purge. Shared by
/// `cli::index_code` and `cli::ingest_code` — every writer that refreshes
/// `indexed_files` cursors must also stamp, or the next `index-code` run
/// wipes the cursors (and with them the BYO `code_vec` rows) it left.
pub(crate) fn stamp_repo_format(conn: &Connection, repo: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO schema_meta(key, value) VALUES(?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![repo_format_key(repo), CODE_FORMAT_VERSION],
    )?;
    Ok(())
}

/// Owned column payload for one `code_symbols` row insert.
///
/// Borrowed-reference fields (rather than owned `String`s) keep call sites
/// from cloning when they already hold borrowed data; the helper passes them
/// straight through to `rusqlite::params!`.
pub struct CodeSymbolRow<'a> {
    pub repo: &'a str,
    pub path: &'a str,
    pub blob_oid: &'a str,
    pub symbol: &'a str,
    pub kind: &'a str,
    pub lang: &'a str,
    pub line_start: i64,
    pub line_end: i64,
    pub snippet: &'a str,
    pub simhash: i64,
    /// Rowid of the parent symbol when this row is a cAST chunk child
    /// (`symbol` is then `<parent>#<n>`); `None` for whole symbols and
    /// chunk parents.
    pub parent_id: Option<i64>,
}

/// Delete every prior `code_symbols` row (and its cascaded `code_vec` /
/// `code_fts` siblings) for `(repo, path)` so a fresh ingest pass with a new
/// `blob_oid` doesn't leave stale rows behind. `code_vec` and `code_fts` are
/// vec0 / fts5 virtual tables and do not participate in the SQLite FK
/// cascade, so we explicitly drop their rows via set-based `IN (SELECT ...)`.
///
/// Shared by `cli::index_code` (file walk) and `cli::ingest_code` (JSONL
/// stream) so both paths cannot drift on the purge SQL.
pub fn purge_file_symbols(conn: &Connection, repo: &str, path: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM code_vec WHERE symbol_id IN (\
             SELECT id FROM code_symbols WHERE repo = ?1 AND path = ?2)",
        rusqlite::params![repo, path],
    )?;
    conn.execute(
        "DELETE FROM code_fts WHERE symbol_id IN (\
             SELECT id FROM code_symbols WHERE repo = ?1 AND path = ?2)",
        rusqlite::params![repo, path],
    )?;
    conn.execute(
        "DELETE FROM code_symbols WHERE repo = ?1 AND path = ?2",
        rusqlite::params![repo, path],
    )?;
    Ok(())
}

/// Upsert the `indexed_files` cursor row so the next `index-code` run knows
/// this blob has already been processed for `(repo, path)`. Used by both
/// `cli::index_code` (after walking a file) and `cli::ingest_code` (after the
/// final symbol row for `(repo, path)` lands).
pub fn upsert_indexed_file(conn: &Connection, repo: &str, path: &str, oid: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO indexed_files(repo, path, blob_oid, indexed_at) \
         VALUES(?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
         ON CONFLICT(repo, path) DO UPDATE \
           SET blob_oid = excluded.blob_oid, \
               indexed_at = excluded.indexed_at",
        rusqlite::params![repo, path, oid],
    )?;
    Ok(())
}

/// Upsert the absolute working-tree `root` for `repo` into
/// `repo_marker.root_path`, creating the marker row if `index-code` is the
/// first writer to touch this repo. `root` is the canonicalized `--path`
/// argument (the exact base `code_symbols.path` values are relative to), so
/// `comemory serve` can rejoin `root + path` to resolve a `file:<repo>:<path>`
/// graph node id to a real file on disk. Stored as an absolute path string;
/// `comemory serve` re-canonicalizes it before any containment check.
pub fn upsert_repo_root(conn: &Connection, repo: &str, root: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO repo_marker(repo, root_path) VALUES(?1, ?2) \
         ON CONFLICT(repo) DO UPDATE SET root_path = excluded.root_path",
        rusqlite::params![repo, root],
    )?;
    Ok(())
}

/// Insert one `code_symbols` row and return its newly-assigned primary key.
/// The `indexed_at` column is stamped server-side via `strftime` so callers
/// don't need to format the timestamp themselves.
pub fn insert(conn: &Connection, row: &CodeSymbolRow<'_>) -> Result<i64> {
    let sid = conn.query_row(
        "INSERT INTO code_symbols(\
             repo, path, blob_oid, symbol, kind, lang, \
             line_start, line_end, snippet, simhash, parent_id, indexed_at) \
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
         RETURNING id",
        rusqlite::params![
            row.repo,
            row.path,
            row.blob_oid,
            row.symbol,
            row.kind,
            row.lang,
            row.line_start,
            row.line_end,
            row.snippet,
            row.simhash,
            row.parent_id,
        ],
        |r| r.get::<_, i64>(0),
    )?;
    Ok(sid)
}

/// Bump `access_count`/`last_accessed` on every `code_symbols` row in
/// `ids`, the code-side twin of `retrieval::pipeline`'s `record_access`
/// over `memories`. Shared by `search-code` (the returned hit ids) and
/// `context` (the resolved code-ref ids) so the two self-reinforcement
/// paths cannot drift on the SQL, the timestamp format, or the
/// best-effort contract.
///
/// All ids fold into one `UPDATE ... WHERE id IN (...)` so the bump costs
/// a single statement and waits on `busy_timeout` at most once. The
/// timestamp goes through [`memory_row::iso_format`] so every
/// `last_accessed` writer (memory and code) emits the same string format.
/// Best-effort: an empty id list is a no-op, and any failure is logged
/// and swallowed — a telemetry write must never break the read path.
pub fn record_access(conn: &Connection, ids: &[i64]) {
    if ids.is_empty() {
        return;
    }
    let now = match memory_row::iso_format(OffsetDateTime::now_utc()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "code access tracking skipped: timestamp format failed");
            return;
        }
    };
    let qmarks = crate::store::qmarks(ids.len());
    let sql = format!(
        "UPDATE code_symbols SET access_count = access_count + 1, last_accessed = ? \
         WHERE id IN ({qmarks})"
    );
    let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(ids.len() + 1);
    params.push(&now);
    for id in ids {
        params.push(id);
    }
    if let Err(e) = conn.execute(&sql, params.as_slice()) {
        tracing::warn!(error = %e, hit_count = ids.len(), "code access tracking update failed");
    }
}
