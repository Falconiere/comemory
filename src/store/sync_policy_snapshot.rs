//! `sync_policy_snapshot` row CRUD — the repository policy last loaded for a
//! session key.
//!
//! A managed origin's policy load writes it; an unmanaged engine's session
//! reads it, because nothing in front of that engine can answer a policy
//! request. A key with no snapshot approves nothing.

use std::collections::BTreeMap;

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_exchange::{SyncPolicySnapshot, sync_policy_snapshot as col};
use super::sync_exchange::ExchangeKey;
use crate::prelude::*;

/// One key's loaded policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicySnapshot {
    /// Monotonic policy revision.
    pub revision: i64,
    /// Digest over the revision, allowlist and mappings.
    pub fingerprint: String,
    /// Approved canonical `owner/name` repositories.
    pub allowlist: Vec<String>,
    /// Confirmed label → canonical mappings.
    pub mappings: BTreeMap<String, String>,
    /// RFC3339 time it was loaded.
    pub loaded_at: String,
}

/// The snapshot stored for `key`, if any.
///
/// # Errors
/// Propagates SQLite failures and a stored JSON column that does not decode.
pub fn load(conn: &Connection, key: &ExchangeKey) -> Result<Option<PolicySnapshot>> {
    let row: Option<(i64, String, String, String, String)> = orm::query_optional(
        conn,
        SyncPolicySnapshot::select()
            .columns_typed(&[
                &col::revision,
                &col::fingerprint,
                &col::allowlist_json,
                &col::mappings_json,
                &col::loaded_at,
            ])
            .filter(col::api_url.eq(key.api_url.as_str()))
            .filter(col::workspace_id.eq(key.workspace_id.as_str()))
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    row.map(|(revision, fingerprint, allowlist, mappings, loaded_at)| {
        Ok(PolicySnapshot {
            revision,
            fingerprint,
            allowlist: serde_json::from_str(&allowlist)?,
            mappings: serde_json::from_str(&mappings)?,
            loaded_at,
        })
    })
    .transpose()
}

/// Replace the snapshot stored for `key`.
///
/// # Errors
/// Propagates SQLite and JSON encoding failures.
pub fn save(conn: &Connection, key: &ExchangeKey, snapshot: &PolicySnapshot) -> Result<()> {
    orm::execute(
        conn,
        SyncPolicySnapshot::insert()
            .or_replace()
            .set(&col::api_url, key.api_url.as_str())
            .set(&col::workspace_id, key.workspace_id.as_str())
            .set(&col::revision, snapshot.revision)
            .set(&col::fingerprint, snapshot.fingerprint.as_str())
            .set(
                &col::allowlist_json,
                serde_json::to_string(&snapshot.allowlist)?,
            )
            .set(
                &col::mappings_json,
                serde_json::to_string(&snapshot.mappings)?,
            )
            .set(&col::loaded_at, snapshot.loaded_at.as_str())
            .to_sql(),
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/sync_policy_snapshot.rs"]
mod tests;
