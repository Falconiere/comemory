//! `comemory gc`'s learning-telemetry eviction: raw `retrieval_log` /
//! `feedback_events` rows older than the configured retention window.
//! Counters in `feedback` are permanent; only these two raw event tables
//! age out. Moved out of `maintenance::gc::sweep_learning` (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).
//!
//! Captured candidate observations age out on the same window, but with one
//! extra rule of their own, so they live in
//! `store::candidate_observations::evict_unjudged_before` rather than here: a
//! judged observation is retained however old it is, because evicting it
//! would leave a reviewed verdict with no passage and no content version to
//! verify it against.

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
/// chronologically — see `maintenance::gc::sweep_learning`'s doc for the format
/// note this preserves.
///
/// Before either delete, the journal copies of every shared event past the
/// same cutoff are expired (#254) — the `feedback_events` rows about to go
/// here and the `activity_log` rows `maintenance::gc` evicts right after —
/// in the same transaction, so a shared event's detail never outlives its
/// retention in the journal while its dedupe metadata stays.
pub fn evict_before(conn: &Connection, cutoff: &str) -> Result<(u64, u64)> {
    let at = super::memory_row::iso_format(time::OffsetDateTime::now_utc())?;
    let tx = conn.unchecked_transaction()?;
    super::replica_redaction::expire_events_before(&tx, cutoff, &at)?;
    let logs = tx.execute("DELETE FROM retrieval_log WHERE at < ?1", [cutoff])?;
    let events = tx.execute("DELETE FROM feedback_events WHERE at < ?1", [cutoff])?;
    tx.commit()?;
    Ok((logs as u64, events as u64))
}

#[cfg(test)]
#[path = "tests/gc_learning.rs"]
mod tests;
