//! Transfer bundle writes (#342): a carried table's rows inserted by the
//! columns its declaration names, one statement per row, under foreign keys
//! deferred to commit, then checked per table before the caller commits.
//! Table and column names come from a [`TableShape`], never from a bundle.
//! The caller owns the transaction; nothing here opens one. Exercised with
//! the reads in `tests/project_transfer.rs`.

use rusqlite::{Connection, params_from_iter};
use serde_json::Value as Json;
use toolu_orm::core::alias::TableRef;
use toolu_orm::core::expr::Scalar;
use toolu_orm::core::value::Value;
use toolu_orm::query::insert::InsertBuilder;
use toolu_orm::query::select::SelectBuilder;

use super::orm;
use super::project_table_shape::TableShape;
use crate::prelude::*;

/// What writing a table's rows did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowsInsert {
    /// Every row was written.
    Inserted,
    /// SQLite refused a row with a constraint of this table (a primary key,
    /// unique index, CHECK or NOT NULL); the caller rolls back.
    Conflict,
}

/// Write `rows` into `table`, each row its declared columns in declaration
/// order, with one statement per row. Run [`defer_foreign_keys`] first when
/// a table references itself. A constraint refusing a row is
/// [`RowsInsert::Conflict`]; every other error propagates.
pub fn insert_rows(
    conn: &Connection,
    table: &TableShape,
    rows: &[Vec<Json>],
) -> Result<RowsInsert> {
    let bound = rows
        .iter()
        .map(|row| params_from_iter(row.iter().map(bind)));
    match orm::execute_many(conn, insert_statement(table), bound) {
        Ok(_) => Ok(RowsInsert::Inserted),
        Err(Error::Sqlite(rusqlite::Error::SqliteFailure(ffi, _)))
            if ffi.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Ok(RowsInsert::Conflict)
        }
        Err(e) => Err(e),
    }
}

/// `INSERT INTO <table> (<columns>) SELECT ? AS <c1>, …`: one placeholder per
/// declared column, rebound for every row.
fn insert_statement(table: &TableShape) -> (String, Vec<Value>) {
    let placeholders = table
        .columns
        .iter()
        .fold(SelectBuilder::raw(), |select, column| {
            select.column_scalar(Scalar::raw("?", vec![Value::Null]), &column.name)
        });
    InsertBuilder::into_table(TableRef::new(&table.name))
        .select_raw(&table.names(), placeholders)
        .to_sql()
}

/// One JSON scalar as the value it binds. The bundle has already been
/// validated against the declaration (strings and integers only), so
/// anything else binds `NULL`, which a NOT NULL column still refuses.
fn bind(value: &Json) -> Value {
    match value {
        Json::String(s) => Value::from(s.as_str()),
        Json::Number(n) => n.as_i64().map_or(Value::Null, Value::from),
        _ => Value::Null,
    }
}

/// Defer every foreign key to commit for the rest of this transaction, so a
/// self-referencing table's rows go in whatever order; SQLite switches it
/// off again at `COMMIT` or `ROLLBACK`.
pub fn defer_foreign_keys(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "defer_foreign_keys", true)?;
    Ok(())
}

/// The first of `tables` with a row whose foreign key names no parent, by
/// `PRAGMA foreign_key_check` per table — never the whole database, so a
/// stale violation elsewhere cannot refuse a transfer.
pub fn first_key_violation(conn: &Connection, tables: &[&str]) -> Result<Option<String>> {
    for table in tables {
        let violations: i64 = conn.query_row(
            "SELECT count(*) FROM pragma_foreign_key_check(?1)",
            [table],
            |r| r.get(0),
        )?;
        if violations > 0 {
            return Ok(Some((*table).to_string()));
        }
    }
    Ok(None)
}
