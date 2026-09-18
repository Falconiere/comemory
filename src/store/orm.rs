//! Execute toolu-orm generated statements on the store's existing connection.
//! The builders own SQL generation; this bridge retains native SQLite errors,
//! cached statements, row decoders and caller-owned transaction boundaries.

use rusqlite::{Connection, OptionalExtension, Row, params_from_iter};
use toolu_orm::core::value::Value;

use crate::prelude::*;

/// Report invalid query construction before invoking SQLite.
pub(super) fn build_error(error: toolu_orm::core::error::DbCoreError) -> Error {
    Error::Other(format!("query construction: {error}"))
}

/// Execute generated DML and return the affected row count.
pub(super) fn execute(conn: &Connection, query: (String, Vec<Value>)) -> Result<usize> {
    let (sql, values) = query;
    Ok(conn
        .prepare_cached(&sql)?
        .execute(params_from_iter(values))?)
}

/// Decode all rows from a generated query into owned values.
pub(super) fn query_all<T>(
    conn: &Connection,
    query: (String, Vec<Value>),
    decode: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>> {
    let (sql, values) = query;
    Ok(conn
        .prepare_cached(&sql)?
        .query_map(params_from_iter(values), decode)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Decode one required row, retaining SQLite's missing-row error.
pub(super) fn query_one<T>(
    conn: &Connection,
    query: (String, Vec<Value>),
    decode: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<T> {
    Ok(query_row(conn, query, decode)?)
}

/// Decode an optional row without exposing driver control flow to callers.
pub(super) fn query_optional<T>(
    conn: &Connection,
    query: (String, Vec<Value>),
    decode: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<Option<T>> {
    Ok(query_row(conn, query, decode).optional()?)
}

fn query_row<T>(
    conn: &Connection,
    query: (String, Vec<Value>),
    decode: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    let (sql, values) = query;
    conn.prepare_cached(&sql)?
        .query_row(params_from_iter(values), decode)
}

#[cfg(test)]
#[path = "tests/orm.rs"]
mod tests;
