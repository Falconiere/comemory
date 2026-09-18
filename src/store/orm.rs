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

/// Execute one generated statement repeatedly with row-specific bindings.
/// The query's initial values establish its placeholder shape; each row replaces
/// all of them in the same order. The caller owns any surrounding transaction.
pub(super) fn execute_many<P: rusqlite::Params>(
    conn: &Connection,
    query: (String, Vec<Value>),
    rows: impl IntoIterator<Item = P>,
) -> Result<u64> {
    let mut statement = conn.prepare_cached(&query.0)?;
    let mut written = 0_u64;
    for values in rows {
        let count = statement.execute(values)?;
        written = written.saturating_add(u64::try_from(count).unwrap_or(0));
    }
    Ok(written)
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
