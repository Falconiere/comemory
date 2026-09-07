//! `retrieval_log` reads — the raw `returned_ids` provenance query behind
//! [`crate::graph::search_edit`]'s search→edit lookback. `retrieval_log`
//! rows are written by `retrieval::pipeline::log_retrieval`; this module
//! only reads them back.

use rusqlite::{Connection, params};

use crate::prelude::*;

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
