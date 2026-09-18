//! Per-memory feedback counters: `used` and `irrelevant`.
//!
//! Each record corresponds to one memory id and tracks how many times the
//! memory was surfaced and accepted vs. dismissed. Inserts use SQLite UPSERT
//! so callers do not need to seed rows. The query-id contract (generate +
//! validate) and the persisted provenance vocabulary are shared contracts and
//! live in [`crate::utilities::query_id`] and
//! `crate::utilities::telemetry` (#166).

use time::OffsetDateTime;

use crate::domains::learning::telemetry::StatsDb;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::feedback as store_feedback;
use crate::store::memory_row;
use crate::utilities::telemetry::{PROV_IMPLICIT, PROV_MANUAL};

/// The caller-facing `source` vocabulary of `POST /api/v1/feedback` and
/// `POST /api/v1/search/{query_id}/feedback`, and its one mapping onto the
/// stored `feedback_events.provenance` value. Closed on purpose: a free
/// string would let a client spell `manual` a second way and split the
/// `= 'manual'` test every reader of the column keys on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// A human stated the verdict. Stored as [`PROV_MANUAL`].
    #[default]
    Explicit,
    /// The verdict was observed, not stated. Stored as [`PROV_IMPLICIT`].
    Implicit,
}

impl Source {
    /// Parse the exact lowercase words `explicit` / `implicit`. Anything
    /// else — `manual`, `Implicit`, an empty string — is an
    /// [`Error::BadRequest`] naming the offender (HTTP `400 bad_request`),
    /// the same shape the per-hit route answers for an unknown `signal`.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "explicit" => Ok(Self::Explicit),
            "implicit" => Ok(Self::Implicit),
            other => Err(Error::BadRequest(format!(
                "unknown source `{other}`: expected explicit or implicit"
            ))),
        }
    }

    /// The `feedback_events.provenance` value verdicts of this source are
    /// stored under.
    pub fn provenance(self) -> &'static str {
        match self {
            Self::Explicit => PROV_MANUAL,
            Self::Implicit => PROV_IMPLICIT,
        }
    }
}

/// Mint one implicit `used` for `id` with caller-chosen `provenance` and
/// sentinel `query_id`: bumps the `feedback` counter and writes a
/// memory-target `feedback_events` row. Composes inside the caller's
/// transaction (the co-activation reward runs within materialize's), so it
/// takes a bare [`Connection`] rather than a [`StatsDb`]. The SQL lives in
/// [`store_feedback`].
///
/// `at` is the run timestamp (already `iso_format`-shaped by the caller).
/// Callers pass [`PROV_AUTO_COACTIVATION`] with [`COACTIVATION_QUERY_ID`] or
/// [`PROV_AUTO_SEARCH_EDIT`] with [`SEARCH_EDIT_QUERY_ID`]. The
/// `comemory feedback` / HTTP path is [`record_with_provenance`], which
/// writes [`PROV_MANUAL`] or [`PROV_IMPLICIT`] through the same
/// [`store_feedback::insert_event`].
pub(crate) fn record_implicit_used(
    conn: &Connection,
    id: &str,
    at: &str,
    provenance: &str,
    query_id: &str,
) -> Result<()> {
    store_feedback::insert_event(
        conn,
        query_id,
        id,
        "used",
        at,
        crate::utilities::telemetry::target::MEMORY,
        provenance,
    )?;
    store_feedback::upsert_used(conn, id, at)?;
    Ok(())
}

/// Record a batch of used/irrelevant verdicts for one query in a single
/// transaction: one `feedback_events` row per id plus the matching
/// counter upsert, written together per id. All-or-nothing — a failure
/// on any id leaves both tables untouched, so events and counters
/// cannot drift.
///
/// `provenance` is stamped on every event row of the batch — a
/// [`Source::provenance`] value ([`PROV_MANUAL`] / [`PROV_IMPLICIT`]) from
/// `domains::learning::feedback::run`. Both verdicts carry it, so an implicit *negative*
/// is storable, not only an implicit `used`.
///
/// The query id is recorded verbatim; it is not required to exist in
/// `retrieval_log` (gc may have evicted the row, or the caller may be
/// replaying feedback) — the caller decides whether to warn.
///
/// `feedback_events.at` goes through [`memory_row::iso_format`] — the
/// same writer as `retrieval_log.at` — so gc's lexicographic cutoff
/// comparison stays sound across both tables.
pub fn record_with_provenance(
    db: &mut StatsDb,
    query_id: &str,
    used: &[String],
    irrelevant: &[String],
    provenance: &str,
) -> Result<()> {
    let now = memory_row::iso_format(OffsetDateTime::now_utc())?;
    let tx = db.conn_mut().transaction()?;
    for id in used {
        store_feedback::insert_event(
            &tx,
            query_id,
            id,
            "used",
            &now,
            crate::utilities::telemetry::target::MEMORY,
            provenance,
        )?;
        store_feedback::upsert_used(&tx, id, &now)?;
    }
    for id in irrelevant {
        store_feedback::insert_event(
            &tx,
            query_id,
            id,
            "irrelevant",
            &now,
            crate::utilities::telemetry::target::MEMORY,
            provenance,
        )?;
        store_feedback::upsert_irrelevant(&tx, id)?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/feedback_tracking.rs"]
mod tests;
