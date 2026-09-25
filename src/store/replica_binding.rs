//! `replica_binding` row CRUD — each entity a session key has exchanged, and
//! the revision the upstream last held for it.
//!
//! Two readers depend on it. The pull skips an entry at or below an entity's
//! synced position, so a rewind can never put an older revision over a newer
//! one; verification compares the upstream's buckets against these digests,
//! so a pending local edit does not read as a mismatch.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;
use toolu_orm::query::insert::OnConflict;

use super::orm;
use super::schema_exchange::{ReplicaBinding, replica_binding as col};
use super::sync_exchange::ExchangeKey;
use crate::prelude::*;

/// What the upstream last held for one entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// Entity kind.
    pub entity_kind: String,
    /// Entity key within its kind.
    pub entity_key: String,
    /// Payload digest; `None` for a tombstone.
    pub synced_digest: Option<String>,
    /// Whether that revision is a tombstone.
    pub synced_deleted: bool,
    /// Its upstream position; `None` after a rebootstrap cleared it.
    pub synced_sequence: Option<i64>,
    /// The epoch `synced_sequence` belongs to.
    pub synced_epoch: Option<String>,
}

/// Write `binding` for `key`, replacing the entity's previous one.
///
/// # Errors
/// Propagates SQLite failures.
pub fn upsert(conn: &Connection, key: &ExchangeKey, binding: &Binding, at: &str) -> Result<()> {
    orm::execute(
        conn,
        ReplicaBinding::insert()
            .set(&col::api_url, key.api_url.as_str())
            .set(&col::workspace_id, key.workspace_id.as_str())
            .set(&col::entity_kind, binding.entity_kind.as_str())
            .set(&col::entity_key, binding.entity_key.as_str())
            .set(&col::synced_digest, binding.synced_digest.as_deref())
            .set(&col::synced_deleted, i64::from(binding.synced_deleted))
            .set(&col::synced_sequence, binding.synced_sequence)
            .set(&col::synced_epoch, binding.synced_epoch.as_deref())
            .set(&col::updated_at, at)
            .on_conflict(
                OnConflict::column(&col::api_url)
                    .and_column(&col::workspace_id)
                    .and_column(&col::entity_kind)
                    .and_column(&col::entity_key)
                    .set_excluded(&col::synced_digest)
                    .set_excluded(&col::synced_deleted)
                    .set_excluded(&col::synced_sequence)
                    .set_excluded(&col::synced_epoch)
                    .set_excluded(&col::updated_at),
            )
            .to_sql(),
    )?;
    Ok(())
}

/// The binding `key` holds for one entity.
///
/// # Errors
/// Propagates SQLite failures.
pub fn get(
    conn: &Connection,
    key: &ExchangeKey,
    entity_kind: &str,
    entity_key: &str,
) -> Result<Option<Binding>> {
    Ok(select(conn, key, Some((entity_kind, entity_key)))?.pop())
}

/// Every binding `key` holds, for verification.
///
/// # Errors
/// Propagates SQLite failures.
pub fn all(conn: &Connection, key: &ExchangeKey) -> Result<Vec<Binding>> {
    select(conn, key, None)
}

/// Every key one entity is bound to, most recently bound first — what decides
/// whose data a local edit of it is.
///
/// # Errors
/// Propagates SQLite failures.
pub fn keys_for(
    conn: &Connection,
    entity_kind: &str,
    entity_key: &str,
) -> Result<Vec<ExchangeKey>> {
    orm::query_all(
        conn,
        ReplicaBinding::select()
            .columns_typed(&[&col::api_url, &col::workspace_id])
            .filter(col::entity_kind.eq(entity_kind))
            .filter(col::entity_key.eq(entity_key))
            .order_by(col::updated_at.desc())
            .to_sql(),
        |r| {
            Ok(ExchangeKey {
                api_url: r.get(0)?,
                workspace_id: r.get(1)?,
            })
        },
    )
}

/// Forget every synced position `key` recorded, keeping the digests — what a
/// rebootstrap does, since a replaced stream may reuse positions.
///
/// # Errors
/// Propagates SQLite failures.
pub fn clear_sequences(conn: &Connection, key: &ExchangeKey) -> Result<usize> {
    orm::execute(
        conn,
        ReplicaBinding::update()
            .set(&col::synced_sequence, None::<i64>)
            .set(&col::synced_epoch, None::<&str>)
            .filter(col::api_url.eq(key.api_url.as_str()))
            .filter(col::workspace_id.eq(key.workspace_id.as_str()))
            .to_sql(),
    )
}

/// The shared select, optionally narrowed to one entity.
fn select(
    conn: &Connection,
    key: &ExchangeKey,
    entity: Option<(&str, &str)>,
) -> Result<Vec<Binding>> {
    let mut query = ReplicaBinding::select()
        .columns_typed(&[
            &col::entity_kind,
            &col::entity_key,
            &col::synced_digest,
            &col::synced_deleted,
            &col::synced_sequence,
            &col::synced_epoch,
        ])
        .filter(col::api_url.eq(key.api_url.as_str()))
        .filter(col::workspace_id.eq(key.workspace_id.as_str()));
    if let Some((kind, entity_key)) = entity {
        query = query
            .filter(col::entity_kind.eq(kind))
            .filter(col::entity_key.eq(entity_key));
    }
    orm::query_all(
        conn,
        query
            .order_by(col::entity_kind.asc())
            .order_by(col::entity_key.asc())
            .to_sql(),
        decode,
    )
}

/// One binding, in the order [`select`] projects it.
fn decode(r: &rusqlite::Row<'_>) -> rusqlite::Result<Binding> {
    Ok(Binding {
        entity_kind: r.get(0)?,
        entity_key: r.get(1)?,
        synced_digest: r.get(2)?,
        synced_deleted: r.get::<_, i64>(3)? != 0,
        synced_sequence: r.get(4)?,
        synced_epoch: r.get(5)?,
    })
}

#[cfg(test)]
#[path = "tests/replica_binding.rs"]
mod tests;
