//! `project_transfer_bindings` (#342): the one transfer a project took part
//! in. An upsert, because a later transfer of the same project replaces the
//! record rather than adding history — the epic rejects a mesh, so a project
//! is bound to at most one other side at a time.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;
use toolu_orm::core::table::TableSchema;

use super::orm;
use super::schema_projects::{ProjectTransferBindings, project_transfer_bindings as col};
use crate::prelude::*;

/// One binding row, written or read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingRow {
    /// The bound project.
    pub project_id: String,
    /// `imported` or `exported`.
    pub direction: String,
    /// The other side's label.
    pub remote: String,
    /// The effective digest: the transferred rows after any actor remap.
    pub digest: String,
    /// The `(type, id)` an actor remap replaced, when one ran.
    pub remapped_from: Option<(String, String)>,
    /// Epoch milliseconds.
    pub transferred_at: i64,
}

/// Write `row`, replacing any binding the project already had. `OR REPLACE`
/// is exact here: nothing references a binding, and every column is rewritten.
pub fn upsert(conn: &Connection, row: &BindingRow) -> Result<()> {
    let (from_type, from_id) = row
        .remapped_from
        .as_ref()
        .map_or((None, None), |(t, i)| (Some(t.as_str()), Some(i.as_str())));
    // Every TEXT column with its value; `NULL` only for the remap pair.
    let text = [
        (&col::project_id, Some(row.project_id.as_str())),
        (&col::direction, Some(row.direction.as_str())),
        (&col::remote, Some(row.remote.as_str())),
        (&col::digest, Some(row.digest.as_str())),
        (&col::remapped_from_principal_type, from_type),
        (&col::remapped_from_principal_id, from_id),
    ];
    let insert = text
        .into_iter()
        .fold(ProjectTransferBindings::insert(), |q, (column, value)| {
            q.set(column, value)
        })
        .set(&col::transferred_at, row.transferred_at)
        .or_replace();
    orm::execute(conn, insert.to_sql())?;
    Ok(())
}

/// The binding of `project_id`, when it has one.
pub fn find(conn: &Connection, project_id: &str) -> Result<Option<BindingRow>> {
    // Every declared column, decoded by name below.
    let def = ProjectTransferBindings::table_def();
    let names: Vec<&str> = def.columns.iter().map(|c| c.name.as_str()).collect();
    let query = ProjectTransferBindings::select()
        .columns_raw(&names)
        .filter(col::project_id.eq(project_id));
    orm::query_optional(conn, query.to_sql(), |r| {
        let from_type: Option<String> = r.get(col::remapped_from_principal_type.name)?;
        let from_id: Option<String> = r.get(col::remapped_from_principal_id.name)?;
        Ok(BindingRow {
            project_id: r.get(col::project_id.name)?,
            direction: r.get(col::direction.name)?,
            remote: r.get(col::remote.name)?,
            digest: r.get(col::digest.name)?,
            remapped_from: from_type.zip(from_id),
            transferred_at: r.get(col::transferred_at.name)?,
        })
    })
}

#[cfg(test)]
#[path = "tests/project_binding.rs"]
mod tests;
