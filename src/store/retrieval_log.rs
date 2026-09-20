//! `retrieval_log` insert + reads: the single write behind every tracked
//! `search`/`context`/`search-code`/`find` run
//! (`retrieval::pipeline::log_retrieval`), the raw `returned_ids`
//! provenance query behind [`crate::domains::graph::search_edit`]'s search→edit
//! lookback, and [`pending_since`] / [`count_since`] — the unjudged-query
//! and total-tracked-query window scans behind
//! `domains::learning::recall_status`.

use std::collections::HashSet;

use rusqlite::{Connection, params};
use serde::Serialize;

use super::{
    orm,
    schema_learning::{RetrievalLog, retrieval_log as col},
};
use crate::prelude::*;
use crate::utilities::telemetry::source;
use toolu_orm::core::query_column::CommonOps;

/// Insert parameters for one `retrieval_log` row, bundled into a struct
/// rather than eight positional arguments (`clippy::too_many_arguments`).
pub struct NewLogRow<'a> {
    /// Deterministic query id (`utilities::query_id::generate_query_id`).
    pub query_id: &'a str,
    /// The raw query text.
    pub query: &'a str,
    /// Pre-serialized JSON array of returned ids.
    pub returned_ids: &'a str,
    /// Pre-rendered ISO-8601 UTC timestamp (`store::memory_row::iso_format`).
    pub at: &'a str,
    /// Wall-clock duration of the run, in milliseconds.
    pub duration_ms: i64,
    /// Repo filter the caller searched with, verbatim (`None` → NULL).
    pub repo: Option<&'a str>,
    /// Kind filter the caller searched with (`--lang` for code searches).
    pub kind: Option<&'a str>,
    /// Query origin (a `crate::utilities::telemetry::source` const).
    pub source: &'a str,
}

/// Insert one `retrieval_log` row. The single write behind every tracked
/// run — memory searches, `context`, and code searches (which text-encode
/// their symbol ids so `returned_ids`'s column shape matches the memory
/// rows).
pub fn insert(conn: &Connection, row: &NewLogRow<'_>) -> Result<()> {
    orm::execute(
        conn,
        RetrievalLog::insert()
            .set(&col::query_id, row.query_id)
            .set(&col::query, row.query)
            .set(&col::returned_ids, row.returned_ids)
            .set(&col::at, row.at)
            .set(&col::duration_ms, row.duration_ms)
            .set(&col::repo, row.repo)
            .set(&col::kind, row.kind)
            .set(&col::source, row.source)
            .to_sql(),
    )?;
    Ok(())
}

/// One tracked query with no `feedback_events` verdict yet, decoded for
/// `domains::learning::recall_status`'s `pending` list. Plain owned data
/// (like [`crate::store::eval_runs::EvalRunRow`]), so `recall_status::run`
/// reuses it directly for the JSON boundary rather than mirroring it with a
/// second type.
#[derive(Debug, Serialize)]
pub struct PendingRow {
    /// Deterministic query id.
    pub query_id: String,
    /// The raw query text.
    pub query: String,
    /// ISO-8601 UTC timestamp the query ran at.
    pub at: String,
    /// The `retrieval_log.source` value (`search` / `context` /
    /// `search-code` / `find`).
    pub source: String,
    /// The decoded `returned_ids` JSON array.
    pub returned_ids: Vec<String>,
}

/// Every tracked recall (`search` / `context` / `search-code` / `find`) at
/// or after `since`, optionally scoped to `repo`, with no matching
/// `feedback_events` row for its `query_id` — the `LEFT JOIN … IS NULL`
/// behind `domains::learning::recall_status`'s "pending" list. `since` must
/// already be `iso_format`-shaped, matching `retrieval_log.at`, so the plain
/// string `>=` compares chronologically. Ordered by `at` ascending. Text
/// `>=` and `LEFT JOIN` both became expressible in toolu-orm 0.10.1
/// (`Scalar::gte`, `SelectBuilder::left_join`); this is still hand SQL,
/// awaiting conversion — see `docs/guides/runtime-orm.md`. A malformed
/// `returned_ids` value propagates as [`Error::Json`] rather than being
/// skipped (unlike [`crate::domains::graph::search_edit`]'s best-effort
/// reward scan): a status report must not understate pending recalls to
/// the hook deciding whether to block the session.
pub fn pending_since(
    conn: &Connection,
    repo: Option<&str>,
    since: &str,
) -> Result<Vec<PendingRow>> {
    let mut stmt = conn.prepare(
        "SELECT rl.query_id, rl.query, rl.at, rl.source, rl.returned_ids \
           FROM retrieval_log rl \
           LEFT JOIN feedback_events fe ON fe.query_id = rl.query_id \
          WHERE rl.source IN (?1, ?2, ?3, ?4) \
            AND rl.at >= ?5 \
            AND (?6 IS NULL OR rl.repo = ?6) \
            AND fe.query_id IS NULL \
          ORDER BY rl.at ASC",
    )?;
    let raw_rows = stmt
        .query_map(
            params![
                source::SEARCH,
                source::CONTEXT,
                source::SEARCH_CODE,
                source::FIND,
                since,
                repo
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    raw_rows
        .into_iter()
        .map(|(query_id, query, at, source, returned_ids_raw)| {
            let returned_ids: Vec<String> = serde_json::from_str(&returned_ids_raw)?;
            Ok(PendingRow {
                query_id,
                query,
                at,
                source,
                returned_ids,
            })
        })
        .collect()
}

/// Count of tracked recalls (`search` / `context` / `search-code` / `find`)
/// at or after `since`, optionally scoped to `repo` — pending ∪ judged, the
/// `queries` total behind `domains::learning::recall_status`'s window
/// report. Same source set and `since`/`repo` predicates as
/// [`pending_since`], minus its `LEFT JOIN feedback_events`: unlike that
/// scan, a row here is counted whether or not it has been judged yet.
/// `since` must already be `iso_format`-shaped, matching `retrieval_log.at`,
/// so the plain string `>=` compares chronologically.
pub fn count_since(conn: &Connection, repo: Option<&str>, since: &str) -> Result<u64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM retrieval_log \
          WHERE source IN (?1, ?2, ?3, ?4) \
            AND at >= ?5 \
            AND (?6 IS NULL OR repo = ?6)",
        params![
            source::SEARCH,
            source::CONTEXT,
            source::SEARCH_CODE,
            source::FIND,
            since,
            repo
        ],
        |r| r.get(0),
    )?;
    Ok(count as u64)
}

/// Return the RAW `returned_ids` JSON strings from every `retrieval_log`
/// row whose `source` is `source_a` or `source_b`, whose `at` falls in
/// `[from, to]` inclusive, and where `repo IS NULL OR repo = repo` (an
/// unscoped log row still matches any repo filter).
///
/// Fixed to exactly two sources rather than a variable-length `IN (...)`
/// list: the one caller ([`crate::domains::graph::search_edit::memories_seen_recently`])
/// always queries `(search, context)`, and a generated `IN` list would add
/// complexity with no second caller to justify it.
///
/// Malformed JSON in a returned row is a caller concern — this function
/// returns the string as stored, unparsed.
pub fn returned_ids_in_window(
    conn: &Connection,
    source_a: &str,
    source_b: &str,
    from: &str,
    to: &str,
    repo: Option<&str>,
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT returned_ids FROM retrieval_log \
          WHERE source IN (?1, ?2) \
            AND at >= ?3 AND at <= ?4 \
            AND (repo IS NULL OR repo = ?5)",
    )?;
    let rows = stmt
        .query_map(params![source_a, source_b, from, to, repo], |r| {
            r.get::<_, String>(0)
        })?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(rows)
}

/// One `(query_id, query, at)` row from the reformulation-mining scan.
pub struct LogQueryRow {
    /// Deterministic query id.
    pub query_id: String,
    /// The raw query text.
    pub query: String,
    /// ISO-8601 UTC timestamp the query ran at.
    pub at: String,
}

/// Every `retrieval_log` row whose `source` is not `exclude_source`, ordered
/// `(at, query_id)` — the raw log `eval::mine::mine` scans for (failed →
/// fixed) reformulation pairs.
pub fn queries_excluding_source(
    conn: &Connection,
    exclude_source: &str,
) -> Result<Vec<LogQueryRow>> {
    orm::query_all(
        conn,
        RetrievalLog::select()
            .columns_typed(&[&col::query_id, &col::query, &col::at])
            .filter(col::source.ne(exclude_source))
            .order_by(col::at.asc())
            .order_by(col::query_id.asc())
            .to_sql(),
        |r| {
            Ok(LogQueryRow {
                query_id: r.get(0)?,
                query: r.get(1)?,
                at: r.get(2)?,
            })
        },
    )
}

/// Whether `query_id` names a row in `retrieval_log`.
///
/// `comemory feedback` probes this before recording a verdict: a miss means
/// the run was evicted by retention or never logged, which is worth a warning
/// but never a refusal, so the caller records the verdict either way.
pub fn contains_query_id(conn: &Connection, query_id: &str) -> Result<bool> {
    orm::query_one(
        conn,
        RetrievalLog::select()
            .filter(col::query_id.eq(query_id))
            .to_exists_sql(),
        |r| r.get(0),
    )
}

/// Every `retrieval_log` row whose `source` is not `exclude_source` and
/// whose `query` matches `like_prefix` (an already-escaped `LIKE` pattern,
/// paired with `ESCAPE '\'`), newest first.
pub fn prefix_matches(
    conn: &Connection,
    exclude_source: &str,
    like_prefix: &str,
) -> Result<Vec<LogQueryRow>> {
    let mut out = Vec::new();
    scan_prefix_matches(conn, exclude_source, like_prefix, |row| {
        out.push(row);
        true
    })?;
    Ok(out)
}

/// The newest `limit` distinct prefix matches, retaining the first row's
/// `query_id` and using Rust Unicode lowercase for deduplication. Stops the
/// ordered SQLite cursor once enough distinct queries have been found.
pub fn distinct_prefix_matches(
    conn: &Connection,
    exclude_source: &str,
    like_prefix: &str,
    limit: usize,
) -> Result<Vec<LogQueryRow>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    scan_prefix_matches(conn, exclude_source, like_prefix, |row| {
        if seen.insert(row.query.to_lowercase()) {
            out.push(row);
        }
        out.len() < limit
    })?;
    Ok(out)
}

/// Visit matching rows in index order until `visit` returns false.
fn scan_prefix_matches(
    conn: &Connection,
    exclude_source: &str,
    like_prefix: &str,
    mut visit: impl FnMut(LogQueryRow) -> bool,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT query, query_id, at FROM retrieval_log \
          WHERE source != ?1 AND query LIKE ?2 ESCAPE '\\' \
          ORDER BY at DESC, query_id DESC",
    )?;
    let mut rows = stmt.query(params![exclude_source, like_prefix])?;
    while let Some(row) = rows.next()? {
        if !visit(LogQueryRow {
            query: row.get(0)?,
            query_id: row.get(1)?,
            at: row.get(2)?,
        }) {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/retrieval_log.rs"]
mod tests;
