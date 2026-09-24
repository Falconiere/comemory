//! Per-memory feedback counters: `used` and `irrelevant`.
//!
//! Each record corresponds to one memory id and tracks how many times the
//! memory was surfaced and accepted vs. dismissed. Inserts use SQLite UPSERT
//! so callers do not need to seed rows. The query-id contract (generate +
//! validate) and the persisted provenance vocabulary are shared contracts and
//! live in [`crate::utilities::query_id`] and
//! `crate::utilities::telemetry` (#166).

use time::OffsetDateTime;

use crate::domains::learning::feedback_share::{self, Caller, Judged, Recorded};
use crate::domains::learning::telemetry::StatsDb;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::connection::write_transaction;
use crate::store::feedback::{self as store_feedback, NewFeedbackEvent};
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
///
/// A search→edit reward is journalled for sharing like a stated verdict; a
/// co-activation reward is not (`replica_payload::SHAREABLE_PROVENANCE`).
pub(crate) fn record_implicit_used(
    conn: &Connection,
    id: &str,
    at: &str,
    provenance: &str,
    query_id: &str,
) -> Result<()> {
    write_one(
        conn,
        &Written {
            query_id,
            id,
            verdict: "used",
            at,
            provenance,
            caller: Caller::default(),
        },
    )?;
    store_feedback::upsert_used(conn, id, at)?;
    Ok(())
}

/// One memory-target verdict to write: its event row, then its journal
/// position when it may be shared.
struct Written<'a> {
    query_id: &'a str,
    id: &'a str,
    verdict: &'a str,
    at: &'a str,
    provenance: &'a str,
    caller: Caller<'a>,
}

/// Insert the event row and journal it, in the caller's transaction. The
/// counter is the caller's step: a verdict's two halves differ by verdict.
fn write_one(conn: &Connection, verdict: &Written<'_>) -> Result<()> {
    let row_id = store_feedback::insert_event(
        conn,
        &NewFeedbackEvent {
            query_id: verdict.query_id,
            memory_id: verdict.id,
            verdict: verdict.verdict,
            at: verdict.at,
            target_kind: crate::utilities::telemetry::target::MEMORY,
            provenance: verdict.provenance,
            surface: verdict.caller.surface,
            actor: verdict.caller.actor,
            device: None,
            event_id: None,
        },
    )?;
    feedback_share::journal(
        conn,
        &Recorded {
            row_id,
            query_id: verdict.query_id,
            verdict: verdict.verdict,
            at: verdict.at,
            provenance: verdict.provenance,
            caller: verdict.caller,
            target: Judged::Memory(verdict.id),
        },
    )?;
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
    let tx = write_transaction(db.conn_mut())?;
    write_with_provenance(
        &tx,
        query_id,
        used,
        irrelevant,
        provenance,
        Caller::default(),
    )?;
    tx.commit()?;
    Ok(())
}

/// Write memory feedback rows through a transaction owned by the caller.
/// This is the shared-transaction half used by the mixed memory+code command;
/// [`record_with_provenance`] retains the standalone public transaction.
pub(crate) fn write_with_provenance(
    conn: &Connection,
    query_id: &str,
    used: &[String],
    irrelevant: &[String],
    provenance: &str,
    caller: Caller<'_>,
) -> Result<()> {
    let now = memory_row::iso_format(OffsetDateTime::now_utc())?;
    let batch = Batch {
        query_id,
        now: &now,
        provenance,
        caller,
    };
    write_verdict_group(conn, &batch, used, "used")?;
    write_verdict_group(conn, &batch, irrelevant, "irrelevant")
}

/// What every verdict of one call shares.
struct Batch<'a> {
    query_id: &'a str,
    now: &'a str,
    provenance: &'a str,
    caller: Caller<'a>,
}

/// Write one verdict group in caller order, sharing the batch timestamp.
fn write_verdict_group(
    conn: &Connection,
    batch: &Batch<'_>,
    ids: &[String],
    verdict: &str,
) -> Result<()> {
    let now = batch.now;
    for id in ids {
        write_one(
            conn,
            &Written {
                query_id: batch.query_id,
                id,
                verdict,
                at: now,
                provenance: batch.provenance,
                caller: batch.caller,
            },
        )?;
        if verdict == "used" {
            store_feedback::upsert_used(conn, id, now)?;
        } else {
            store_feedback::upsert_irrelevant(conn, id)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/feedback_tracking.rs"]
mod tests;
