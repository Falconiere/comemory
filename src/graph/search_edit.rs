//! Search→edit lookback: which memories appeared on a recent search/context
//! page and are therefore eligible for `auto_search_edit` provenance when a
//! referenced file is touched in the co-activation harvest.
//!
//! Reads `retrieval_log.returned_ids` (JSON array written by
//! `pipeline::log_retrieval`). Malformed JSON rows are skipped with a warn —
//! one bad log row must not abort the whole materialize transaction.

use std::collections::HashSet;

use rusqlite::Connection;
use time::{Duration, OffsetDateTime, format_description::well_known::Iso8601};

use crate::prelude::*;
use crate::stats::source;
use crate::store::memory_row;

/// Return the subset of `candidates` that appear in any `retrieval_log` row
/// with `source IN ('search','context')`, `at` in `[cutoff, at]`, and
/// `repo IS NULL OR repo = repo` (unscoped searches still credit).
///
/// `at` must be `iso_format`-shaped. `lookback_days` is the inclusive window
/// length (`≥ 1` by config validation). Empty `candidates` → empty set.
pub(crate) fn memories_seen_recently(
    conn: &Connection,
    repo: &str,
    candidates: &HashSet<String>,
    at: &str,
    lookback_days: u32,
) -> Result<HashSet<String>> {
    if candidates.is_empty() {
        return Ok(HashSet::new());
    }
    let cutoff = lookback_cutoff(at, lookback_days)?;
    let mut stmt = conn.prepare(
        "SELECT returned_ids FROM retrieval_log \
          WHERE source IN (?1, ?2) \
            AND at >= ?3 AND at <= ?4 \
            AND (repo IS NULL OR repo = ?5)",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![source::SEARCH, source::CONTEXT, cutoff, at, repo],
        |r| r.get::<_, String>(0),
    )?;
    let mut hit: HashSet<String> = HashSet::new();
    for row in rows {
        let raw = row?;
        let ids: Vec<String> = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "search-edit lookback: skipping malformed retrieval_log.returned_ids"
                );
                continue;
            }
        };
        for id in ids {
            if candidates.contains(&id) {
                hit.insert(id);
            }
        }
    }
    Ok(hit)
}

/// Subtract `lookback_days` from an `iso_format` timestamp, returning the
/// cutoff as another `iso_format` string (lexicographic compare stays sound).
fn lookback_cutoff(at: &str, lookback_days: u32) -> Result<String> {
    let parsed = OffsetDateTime::parse(at, &Iso8601::DEFAULT)
        .map_err(|e| Error::Other(format!("search-edit lookback: cannot parse at={at}: {e}")))?;
    memory_row::iso_format(parsed - Duration::days(i64::from(lookback_days)))
}
