//! `comemory gc`'s learning-telemetry eviction: raw `retrieval_log` /
//! `feedback_events` rows older than the configured retention window.
//! Counters in `feedback` are permanent; only these two raw event tables
//! age out. Moved out of `api::gc::sweep_learning` (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).

use rusqlite::Connection;

use crate::prelude::*;

/// Delete every `retrieval_log` / `feedback_events` row whose `at` is
/// strictly before `cutoff` (exclusive — a row exactly AT `cutoff` survives
/// this sweep and is evicted only once a later run's cutoff passes it).
/// Returns `(retrieval_log rows deleted, feedback_events rows deleted)`.
///
/// `cutoff` must already be rendered in the same fixed-width ISO-8601 UTC
/// format both tables' `at` columns are written in
/// (`memory_row::iso_format`), so the plain string `<` compares
/// chronologically — see `api::gc::sweep_learning`'s doc for the format
/// note this preserves.
pub fn evict_before(conn: &Connection, cutoff: &str) -> Result<(u64, u64)> {
    let logs = conn.execute("DELETE FROM retrieval_log WHERE at < ?1", [cutoff])?;
    let events = conn.execute("DELETE FROM feedback_events WHERE at < ?1", [cutoff])?;
    Ok((logs as u64, events as u64))
}

#[cfg(test)]
#[path = "tests/gc_learning.rs"]
mod tests;
