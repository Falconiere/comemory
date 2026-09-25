//! `replica_replay` row CRUD — the scratch rows of a compacting replay: each
//! entity's LAST position in the replayed range.
//!
//! The scan phase offers every entry; only a later position replaces an
//! earlier one, so when the scan ends the table holds exactly one entry per
//! entity. The apply phase then takes them in position order and deletes each
//! row in the transaction that applies it, which is what lets a killed replay
//! resume where it stopped.

use rusqlite::Connection;
use toolu_orm::core::expr::Expr;
use toolu_orm::core::query_column::CommonOps;
use toolu_orm::query::insert::OnConflict;

use super::orm;
use super::schema_exchange::{ReplicaReplay, replica_replay as col};
use super::sync_exchange::ExchangeKey;
use crate::prelude::*;

/// One entity's last position in the replayed range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayRow {
    /// Entity kind.
    pub entity_kind: String,
    /// Entity key within its kind.
    pub entity_key: String,
    /// Upstream position of the entry.
    pub sequence: i64,
    /// The entry as the upstream sent it, JSON.
    pub entry_json: String,
}

/// Offer `row`: kept when the entity has no row yet or `row` is later than the
/// one it has. Returns whether it was kept.
///
/// # Errors
/// Propagates SQLite failures.
pub fn offer(conn: &Connection, key: &ExchangeKey, row: &ReplayRow) -> Result<bool> {
    // Only a later position replaces the one held.
    let later = Expr::raw(
        "excluded.\"sequence\" > \"replica_replay\".\"sequence\"",
        Vec::new(),
    );
    let keep_later = OnConflict::column(&col::api_url)
        .and_column(&col::workspace_id)
        .and_column(&col::entity_kind)
        .and_column(&col::entity_key)
        .set_excluded(&col::sequence)
        .set_excluded(&col::entry_json)
        .where_update(later);
    let insert = ReplicaReplay::insert()
        .set(&col::api_url, key.api_url.as_str())
        .set(&col::workspace_id, key.workspace_id.as_str())
        .set(&col::entity_kind, row.entity_kind.as_str())
        .set(&col::entity_key, row.entity_key.as_str())
        .set(&col::sequence, row.sequence)
        .set(&col::entry_json, row.entry_json.as_str())
        .on_conflict(keep_later);
    Ok(orm::execute(conn, insert.to_sql())? > 0)
}

/// The next `limit` rows to apply, lowest position first.
///
/// # Errors
/// Propagates SQLite failures.
pub fn next(conn: &Connection, key: &ExchangeKey, limit: usize) -> Result<Vec<ReplayRow>> {
    let mine = col::api_url
        .eq(key.api_url.as_str())
        .and(col::workspace_id.eq(key.workspace_id.as_str()));
    let lowest_first = ReplicaReplay::select()
        .columns_typed(COLUMNS)
        .filter(mine)
        .order_by(col::sequence.asc())
        .limit(i64::try_from(limit).unwrap_or(i64::MAX));
    orm::query_all(conn, lowest_first.to_sql(), decode)
}

/// Every column [`decode`] reads, in order.
const COLUMNS: &[&dyn toolu_orm::core::query_column::ColumnRef] = &[
    &col::entity_kind,
    &col::entity_key,
    &col::sequence,
    &col::entry_json,
];

/// One scratch row, in [`COLUMNS`] order.
fn decode(r: &rusqlite::Row<'_>) -> rusqlite::Result<ReplayRow> {
    Ok(ReplayRow {
        entity_kind: r.get(0)?,
        entity_key: r.get(1)?,
        sequence: r.get(2)?,
        entry_json: r.get(3)?,
    })
}

/// Drop scratch rows of `key`: one entity's — inside the transaction that
/// applied it — or, with `None`, all of them (a replay starting over).
///
/// # Errors
/// Propagates SQLite failures.
pub fn clear(conn: &Connection, key: &ExchangeKey, entity: Option<(&str, &str)>) -> Result<usize> {
    let mut delete = ReplicaReplay::delete()
        .filter(col::api_url.eq(key.api_url.as_str()))
        .filter(col::workspace_id.eq(key.workspace_id.as_str()));
    if let Some((kind, entity_key)) = entity {
        delete = delete
            .filter(col::entity_kind.eq(kind))
            .filter(col::entity_key.eq(entity_key));
    }
    orm::execute(conn, delete.to_sql())
}

#[cfg(test)]
#[path = "tests/replica_replay.rs"]
mod tests;
