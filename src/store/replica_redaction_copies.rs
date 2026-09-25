//! Redaction reach beyond `replica_payload` (#256, B-6): a payload's blanked
//! bytes must not survive as a second copy in the replay scratch a killed
//! pull left behind. [`super::replica_sweep`] already reaches a stray
//! complete staged-part set on its own 24-hour window, in the same `gc` run
//! that calls this.
//!
//! `replica_replay.entry_json` is the pulled entry verbatim, payload bytes
//! included — a store-layer module cannot name the domain type that shape
//! belongs to, so the digest is read out of the generic JSON instead.

use std::collections::HashSet;

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_exchange::{ReplicaReplay, replica_replay as col};
use crate::prelude::*;

/// One scratch row's composite key and the digest its entry names, if any.
struct ReplayCopy {
    api_url: String,
    workspace_id: String,
    entity_kind: String,
    entity_key: String,
    digest: Option<String>,
}

/// Delete every `replica_replay` scratch row whose entry names one of
/// `digests` — a SIGKILLed replay's private copy of bytes retention just
/// expired. A no-op for an empty list, and for a row whose `entry_json`
/// fails to parse (left alone rather than guessed at). Returns rows cleared.
///
/// # Errors
/// Propagates SQLite failures.
pub fn clear_replay_of(conn: &Connection, digests: &[String]) -> Result<u64> {
    if digests.is_empty() {
        return Ok(0);
    }
    let wanted: HashSet<&str> = digests.iter().map(String::as_str).collect();
    let mut cleared = 0u64;
    for copy in scan(conn)? {
        if copy.digest.as_deref().is_some_and(|d| wanted.contains(d)) {
            orm::execute(
                conn,
                ReplicaReplay::delete()
                    .filter(col::api_url.eq(copy.api_url.as_str()))
                    .filter(col::workspace_id.eq(copy.workspace_id.as_str()))
                    .filter(col::entity_kind.eq(copy.entity_kind.as_str()))
                    .filter(col::entity_key.eq(copy.entity_key.as_str()))
                    .to_sql(),
            )?;
            cleared += 1;
        }
    }
    Ok(cleared)
}

/// Every column [`decode`] reads, in order.
const COLUMNS: &[&dyn toolu_orm::core::query_column::ColumnRef] = &[
    &col::api_url,
    &col::workspace_id,
    &col::entity_kind,
    &col::entity_key,
    &col::entry_json,
];

/// Every scratch row's key and the digest its `entry_json` names, if any.
/// The table holds only entities an in-progress replay has not yet
/// resolved, so a full scan costs nothing a bounded pull would not already.
fn scan(conn: &Connection) -> Result<Vec<ReplayCopy>> {
    orm::query_all(
        conn,
        ReplicaReplay::select().columns_typed(COLUMNS).to_sql(),
        decode,
    )
}

/// One scratch row, in [`COLUMNS`] order.
fn decode(r: &rusqlite::Row<'_>) -> rusqlite::Result<ReplayCopy> {
    let entry_json: String = r.get(4)?;
    Ok(ReplayCopy {
        api_url: r.get(0)?,
        workspace_id: r.get(1)?,
        entity_kind: r.get(2)?,
        entity_key: r.get(3)?,
        digest: digest_of(&entry_json),
    })
}

/// The `payload_digest` field of a scratch row's `entry_json`, when it
/// parses as an object carrying one.
fn digest_of(entry_json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(entry_json).ok()?;
    value
        .get("payload_digest")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

#[cfg(test)]
#[path = "tests/replica_redaction_copies.rs"]
mod tests;
