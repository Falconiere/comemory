//! `retrieval_log` insert + reads: the single write behind every tracked
//! `search`/`context`/`search-code` run
//! (`retrieval::pipeline::log_retrieval`), and the raw `returned_ids`
//! provenance query behind [`crate::graph::search_edit`]'s search→edit
//! lookback.

use rusqlite::{Connection, params};

use crate::prelude::*;

/// Insert parameters for one `retrieval_log` row, bundled into a struct
/// rather than eight positional arguments (`clippy::too_many_arguments`).
pub struct NewLogRow<'a> {
    /// Deterministic query id (`stats::feedback::generate_query_id`).
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
    /// Query origin (a [`crate::stats::source`] const).
    pub source: &'a str,
}

/// Insert one `retrieval_log` row. The single write behind every tracked
/// run — memory searches, `context`, and code searches (which text-encode
/// their symbol ids so `returned_ids`'s column shape matches the memory
/// rows).
pub fn insert(conn: &Connection, row: &NewLogRow<'_>) -> Result<()> {
    conn.execute(
        "INSERT INTO retrieval_log(query_id, query, returned_ids, at, duration_ms,
                                   repo, kind, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            row.query_id,
            row.query,
            row.returned_ids,
            row.at,
            row.duration_ms,
            row.repo,
            row.kind,
            row.source,
        ],
    )?;
    Ok(())
}

/// Return the RAW `returned_ids` JSON strings from every `retrieval_log`
/// row whose `source` is `source_a` or `source_b`, whose `at` falls in
/// `[from, to]` inclusive, and where `repo IS NULL OR repo = repo` (an
/// unscoped log row still matches any repo filter).
///
/// Fixed to exactly two sources rather than a variable-length `IN (...)`
/// list: the one caller ([`crate::graph::search_edit::memories_seen_recently`])
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

#[cfg(test)]
#[path = "tests/retrieval_log.rs"]
mod tests;
