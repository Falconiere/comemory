//! `project_command_receipts` reads and writes (#327): the stored answer to
//! one idempotent project command, found by its `(principal_type,
//! principal_id, idempotency_key)` scope and written inside the command's own
//! transaction. The replay-or-conflict policy is
//! `domains::projects::receipt`'s; a receipt lives until its project is
//! hard-deleted, when the foreign key cascades it away.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_project_record::{ProjectCommandReceipts, project_command_receipts as col};
use crate::prelude::*;

/// The stored fields a replay compares and returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptRow {
    /// Receipt UUID.
    pub id: String,
    /// The command it answers (`project.create`, …).
    pub command_type: String,
    /// Hex SHA-256 of the command type and its body.
    pub request_digest: String,
    /// The stored JSON response.
    pub response: String,
}

/// One new receipt row.
pub struct NewReceipt<'a> {
    /// Receipt UUID.
    pub id: &'a str,
    /// The project the command touched.
    pub project_id: &'a str,
    /// Issuing principal kind.
    pub principal_type: &'a str,
    /// Issuing principal id.
    pub principal_id: &'a str,
    /// The caller's key.
    pub idempotency_key: &'a str,
    /// The command it answers.
    pub command_type: &'a str,
    /// Hex SHA-256 of the command type and its body.
    pub request_digest: &'a str,
    /// The JSON response to replay.
    pub response: &'a str,
    /// Epoch milliseconds.
    pub at_ms: i64,
}

/// The receipt `principal_type`/`principal_id` holds under `key`, if any.
pub fn find(
    conn: &Connection,
    principal_type: &str,
    principal_id: &str,
    key: &str,
) -> Result<Option<ReceiptRow>> {
    orm::query_optional(
        conn,
        ProjectCommandReceipts::select()
            .columns_typed(&[
                &col::id,
                &col::command_type,
                &col::request_digest,
                &col::response,
            ])
            .filter(col::principal_type.eq(principal_type))
            .filter(col::principal_id.eq(principal_id))
            .filter(col::idempotency_key.eq(key))
            .to_sql(),
        |r| {
            Ok(ReceiptRow {
                id: r.get(0)?,
                command_type: r.get(1)?,
                request_digest: r.get(2)?,
                response: r.get(3)?,
            })
        },
    )
}

/// Write `row`. The caller holds the write transaction and has already
/// checked the scope is free, so a unique violation here is a real error.
pub fn insert(conn: &Connection, row: &NewReceipt<'_>) -> Result<()> {
    orm::execute(
        conn,
        ProjectCommandReceipts::insert()
            .set(&col::id, row.id)
            .set(&col::project_id, row.project_id)
            .set(&col::principal_type, row.principal_type)
            .set(&col::principal_id, row.principal_id)
            .set(&col::idempotency_key, row.idempotency_key)
            .set(&col::command_type, row.command_type)
            .set(&col::request_digest, row.request_digest)
            .set(&col::response, row.response)
            .set(&col::created_at, row.at_ms)
            .to_sql(),
    )?;
    Ok(())
}
