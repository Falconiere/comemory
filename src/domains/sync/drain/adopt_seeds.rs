//! Adopting local-origin memory seed positions the outbox never saw (#256):
//! `replica::bootstrap` discards a seed's outbox row unless this engine is
//! already a client, so an engine that seeded before logging in has memories
//! nobody sends. Each seed position whose entity has no outbox row in any
//! state and no binding is enqueued once; a later local operation on the
//! same entity carries it instead, never overtaken by the older seed. Runs
//! once a pass's cursor reaches the captured head, so it never competes with
//! a large backlog pull for the pass's budget.

use crate::domains::memories::replica_payload::MEMORY_ENTITY_KIND;
use crate::prelude::*;
use crate::store::replica_journal::{NewOperation, PayloadRef, ReplicaOrigin};
use crate::store::sync_exchange::ExchangeKey;
use crate::store::{Connection, replica_binding, replica_outbox, replica_read, schema_meta};

/// Feed positions read per page while adopting.
const PAGE: usize = 500;

/// `schema_meta` key prefix; `<prefix>:<api_url>:<workspace_id>` holds the
/// last feed position adoption read for that key.
pub const THROUGH_KEY: &str = "replica_seed_adopted";

/// Queue every unbound, outbox-less memory seed position above this key's
/// adoption cursor. Each page commits with the cursor that covers it, so an
/// interrupted pass resumes where it stopped; a cursor a rebuild resets costs
/// a rescan, never a second upload — the same idempotency
/// [`super::adopt::all`] relies on.
///
/// # Errors
/// Propagates SQLite failures.
pub fn advance(conn: &mut Connection, key: &ExchangeKey, at: &str) -> Result<usize> {
    let cursor_key = format!("{THROUGH_KEY}:{}:{}", key.api_url, key.workspace_id);
    let mut since = schema_meta::get(conn, &cursor_key)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut adopted = 0;
    loop {
        let tx = conn.transaction()?;
        let Some((queued, through)) = page(&tx, since, key, at)? else {
            return Ok(adopted);
        };
        schema_meta::upsert(&tx, &cursor_key, &through.to_string())?;
        tx.commit()?;
        adopted += queued;
        since = through;
    }
}

/// Queue the unbound, outbox-less local memory positions of one page above
/// `since`; `None` when the feed has none above it, else how many were
/// queued and the last position read.
fn page(
    conn: &Connection,
    since: i64,
    key: &ExchangeKey,
    at: &str,
) -> Result<Option<(usize, i64)>> {
    let rows = replica_read::page(conn, since, PAGE, Some(MEMORY_ENTITY_KIND))?;
    let Some(through) = rows.last().map(|row| row.sequence) else {
        return Ok(None);
    };
    let mut queued = 0;
    for row in rows {
        if row.origin != ReplicaOrigin::Local
            || replica_outbox::has_any_for(conn, &row.entity_kind, &row.entity_key)?
            || replica_binding::get(conn, key, &row.entity_kind, &row.entity_key)?.is_some()
        {
            continue;
        }
        let payload = row
            .payload_digest
            .as_deref()
            .map(|digest| PayloadRef { digest, bytes: "" });
        replica_outbox::enqueue(
            conn,
            &NewOperation {
                operation_id: &row.operation_id,
                entity_kind: &row.entity_kind,
                entity_key: &row.entity_key,
                op: row.op,
                payload,
                schema_version: row.schema_version,
                repository: row.repository.as_deref(),
                origin: ReplicaOrigin::Local,
                at,
            },
            None,
        )?;
        queued += 1;
    }
    Ok(Some((queued, through)))
}

#[cfg(test)]
#[path = "tests/adopt_seeds.rs"]
mod tests;
