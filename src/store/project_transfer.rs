//! Transfer bundle reads (#342): one project's rows of any carried table, by
//! the columns its declaration names, so a column added later travels without
//! new code, and the project holding a key prefix or slug. Table and column
//! names always come from a [`TableShape`] built off the declarations, never
//! from a bundle. The caller owns the transaction; nothing here opens one.
//! The import's writes are [`super::project_transfer_write`].

use rusqlite::Connection;
use serde_json::Value as Json;
use toolu_orm::core::alias::TableRef;
use toolu_orm::core::expr::Expr;
use toolu_orm::core::query_column::CommonOps;
use toolu_orm::core::value::Value;
use toolu_orm::query::select::SelectBuilder;

use super::orm;
use super::project_table_shape::TableShape;
use super::schema_projects::{self, projects as col};
use crate::prelude::*;

/// Which unique `projects` identity a lookup names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Identity {
    /// `projects.key_prefix`.
    KeyPrefix,
    /// `projects.slug`.
    Slug,
}

/// Every row of `table` that belongs to `project_id`, each row its declared
/// columns' values in declaration order: TEXT as a string, INTEGER as a
/// number, NULL as `null`. Row order is unspecified; the bundle sorts.
pub fn rows(conn: &Connection, table: &TableShape, project_id: &str) -> Result<Vec<Vec<Json>>> {
    let query = SelectBuilder::from_table(TableRef::new(&table.name))
        .columns_raw(&table.names())
        .filter(scope(table, project_id)?);
    let width = table.columns.len();
    let raw = orm::query_all(conn, query.to_sql(), |row| {
        (0..width)
            .map(|i| row.get(i))
            .collect::<rusqlite::Result<Vec<_>>>()
    })?;
    raw.into_iter()
        .map(|row| row.into_iter().map(|v| json(v, &table.name)).collect())
        .collect()
}

/// The predicate that keeps `table`'s rows of one project: `projects` by its
/// id, a table with `project_id` by that, and a table without one (evidence
/// criteria) through the first parent it references that has one.
fn scope(table: &TableShape, project_id: &str) -> Result<Expr> {
    let bind = || vec![Value::from(project_id)];
    if table.name == "projects" {
        return Ok(Expr::raw(format!("\"{}\" = ?", col::id.name), bind()));
    }
    if table.position("project_id").is_some() {
        return Ok(Expr::raw("\"project_id\" = ?", bind()));
    }
    let defs = schema_projects::table_defs();
    for reference in &table.references {
        let ([column], [key]) = (reference.columns.as_slice(), reference.target.as_slice()) else {
            continue;
        };
        let parent = &reference.table;
        let owned = defs
            .iter()
            .any(|d| d.name == *parent && d.find_column("project_id").is_some());
        if owned {
            let sql = format!(
                "\"{column}\" IN (SELECT \"{key}\" FROM \"{parent}\" WHERE \"project_id\" = ?)"
            );
            return Ok(Expr::raw(sql, bind()));
        }
    }
    Err(Error::Other(format!(
        "{} reaches no project: no project_id and no parent with one",
        table.name
    )))
}

/// One stored value as JSON; a BLOB or a non-finite REAL has no place in a
/// project table, so it is refused rather than reshaped.
fn json(value: rusqlite::types::Value, table: &str) -> Result<Json> {
    use rusqlite::types::Value as Sql;
    match value {
        Sql::Null => Ok(Json::Null),
        Sql::Integer(i) => Ok(Json::from(i)),
        Sql::Text(s) => Ok(Json::from(s)),
        Sql::Real(f) => serde_json::Number::from_f64(f)
            .map(Json::Number)
            .ok_or_else(|| Error::Other(format!("{table} holds a non-finite REAL"))),
        Sql::Blob(_) => Err(Error::Other(format!("{table} holds a BLOB"))),
    }
}

/// The id of the project holding `value` as its `identity`, if any.
pub fn identity_holder(
    conn: &Connection,
    identity: Identity,
    value: &str,
) -> Result<Option<String>> {
    let filter = match identity {
        Identity::KeyPrefix => col::key_prefix.eq(value),
        Identity::Slug => col::slug.eq(value),
    };
    let query = schema_projects::Projects::select()
        .columns_typed(&[&col::id])
        .filter(filter);
    orm::query_optional(conn, query.to_sql(), |r| r.get(0))
}

#[cfg(test)]
#[path = "tests/project_transfer.rs"]
mod tests;
