//! Journal seeding for memories that predate the journal.
//!
//! Seeding advances in bounded batches inside the replica cores themselves —
//! no scheduler, no daemon, no separate command. Each call seeds at most
//! [`BATCH`] memories in ascending id order and records how far
//! it got, so an interrupted run resumes and a concurrent save is unaffected:
//! a new write appends at the head like any other mutation.
//!
//! `replica-v1` is not advertised until the state reaches `complete`, so a
//! peer never treats a half-seeded feed as the whole history.

use crate::domains::memories::{MemoryStore, journal};
use crate::prelude::*;
use crate::store::replica_journal::{ReplicaOp, ReplicaOrigin};
use crate::store::{memory_row, replica_outbox, replica_read, schema_meta, seed_scan};
use crate::utilities::context::Ctx;

/// Memories seeded per call.
const BATCH: usize = 200;

/// `schema_meta` key holding `pending`, `seeding` or `complete`.
pub const STATE_KEY: &str = "replica_bootstrap_state";

/// `schema_meta` key holding the last seeded memory id — the resume point.
pub const THROUGH_KEY: &str = "replica_bootstrap_through";

/// Seeding is finished and the protocol may be advertised.
pub const STATE_COMPLETE: &str = "complete";

/// How far seeding has progressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// `pending`, `seeding` or `complete`.
    pub state: String,
    /// Last seeded memory id; empty before the first batch.
    pub through: String,
}

impl Progress {
    /// Whether the journal describes every live memory.
    #[must_use]
    pub fn complete(&self) -> bool {
        self.state == STATE_COMPLETE
    }
}

/// Read the stored progress.
///
/// # Errors
/// Propagates SQLite failures.
pub fn progress(ctx: &mut Ctx<'_>) -> Result<Progress> {
    let conn = ctx.conn()?;
    let state = schema_meta::get(conn, STATE_KEY)?.unwrap_or_else(|| "pending".to_string());
    let through = schema_meta::get(conn, THROUGH_KEY)?.unwrap_or_default();
    Ok(Progress { state, through })
}

/// Seed up to [`BATCH`] unjournalled memories and return the new progress.
///
/// Idempotent: a memory that already has a revision row is skipped, so a
/// second run over the same range adds nothing.
///
/// # Errors
/// Propagates markdown and SQLite failures.
pub fn advance(ctx: &mut Ctx<'_>) -> Result<Progress> {
    let current = progress(ctx)?;
    if current.complete() {
        return Ok(current);
    }
    let batch = {
        let conn = ctx.conn()?;
        seed_scan::live_memories_after(conn, &current.through, BATCH)?
    };
    let Some(last) = batch.last().cloned() else {
        return mark(ctx, STATE_COMPLETE, &current.through);
    };
    // A short batch is the tail of the scan: finishing here means the
    // capability appears as soon as the journal is whole, rather than one
    // request later.
    let finished = batch.len() < BATCH;
    for id in batch {
        seed_one(ctx, &id)?;
    }
    mark(
        ctx,
        if finished { STATE_COMPLETE } else { "seeding" },
        &last,
    )
}

/// Journal one pre-existing memory as a local upsert, unless it already has a
/// revision.
fn seed_one(ctx: &mut Ctx<'_>, id: &str) -> Result<()> {
    {
        let conn = ctx.conn()?;
        if replica_read::revision(conn, "memory", id)?.is_some() {
            return Ok(());
        }
    }
    let record = match MemoryStore::new(ctx.paths.clone()).load(id) {
        Ok(record) => record,
        // A row whose markdown is gone is a repair job for `doctor`, not a
        // reason to stall seeding for every other memory.
        Err(Error::NotFound(_)) => return Ok(()),
        Err(e) => return Err(e),
    };
    let at = memory_row::iso_format(record.frontmatter.created)?;
    let operation_id = journal::mint_operation_id(id, ReplicaOp::Upsert);
    let conn = ctx.conn()?;
    let tx = conn.transaction()?;
    journal::record_write(
        &tx,
        ReplicaOp::Upsert,
        &record.frontmatter,
        &record.body,
        &at,
        ReplicaOrigin::Local,
        Some(&operation_id),
    )?;
    // Seeding records what this engine already holds; it owes nobody an
    // upload, and an owed upload makes an engine refuse every import for the
    // entity. The journal enqueues every local write, so the row goes in the
    // same transaction that wrote it.
    replica_outbox::discard(&tx, &operation_id)?;
    tx.commit()?;
    Ok(())
}

/// Store the progress markers.
fn mark(ctx: &mut Ctx<'_>, state: &str, through: &str) -> Result<Progress> {
    let conn = ctx.conn()?;
    schema_meta::upsert(conn, STATE_KEY, state)?;
    schema_meta::upsert(conn, THROUGH_KEY, through)?;
    Ok(Progress {
        state: state.to_string(),
        through: through.to_string(),
    })
}

#[cfg(test)]
#[path = "tests/bootstrap.rs"]
mod tests;
