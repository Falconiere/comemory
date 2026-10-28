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

/// The four query shapes over one prepare/bind path. A table keeps them from
/// pairing as near-duplicates while each name still encodes its collect
/// contract (`Vec`, required row, optional row, raw `rusqlite::Result`).
macro_rules! orm_queries {
    () => {
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
    };
}
orm_queries!();

/// Emit read-only store functions from a table of rows. Each row is
/// `name(arg: Ty, ..) -> Ret = runner(query, decode)`: it runs one generated
/// statement (`builder.to_sql()` / `.to_count_sql()` / `.to_exists_sql()`) through the named runner (`query_all` / `query_optional` /
/// `query_one`) with a row decoder, so readers that differ only in
/// arguments, filters and result shape stay one declaration each. A row may
/// add `, empty_on slice_arg` before `=`: an empty slice then short-circuits
/// to `Ret::default()` instead of building an empty `IN ()`.
macro_rules! reads {
    (
        $($(#[$doc:meta])* $vis:vis $name:ident($($arg:ident: $aty:ty),*) -> $ret:ty
            $(, empty_on $empty:ident)? = $runner:ident($query:expr, $decode:expr);)+
    ) => {
        $(
            $(#[$doc])*
            $vis fn $name(
                conn: &rusqlite::Connection $(, $arg: $aty)*
            ) -> $crate::prelude::Result<$ret> {
                $(if $empty.is_empty() {
                    return Ok(Default::default());
                })?
                $crate::store::orm::$runner(conn, $query, $decode)
            }
        )+
    };
}
pub(super) use reads;

/// Emit store functions that run one generated DML statement and discard the
/// affected-row count. Each row is `name(arg: Ty, ..) = query;`, so deleters
/// and flag updaters differ only in their arguments and statement.
macro_rules! writes {
    ($($(#[$doc:meta])* $vis:vis $name:ident($($arg:ident: $aty:ty),*) = $query:expr;)+) => {
        $(
            $(#[$doc])*
            $vis fn $name(
                conn: &rusqlite::Connection $(, $arg: $aty)*
            ) -> $crate::prelude::Result<()> {
                $crate::store::orm::execute(conn, $query.to_sql())?;
                Ok(())
            }
        )+
    };
}
pub(super) use writes;

/// Emit store functions that run one hand-written parameterised SQL
/// statement and discard the affected-row count. Each row is
/// `name(arg: Ty, ..) = SQL, [param, ..];`, with the params bound to
/// `?1..?N` in order, so raw upserts differ only in arguments, SQL and params.
macro_rules! sql_writes {
    (
        $($(#[$doc:meta])* $vis:vis $name:ident($($arg:ident: $aty:ty),*) =
            $sql:expr, [$($param:expr),+ $(,)?];)+
    ) => {
        $(
            $(#[$doc])*
            $vis fn $name(
                conn: &rusqlite::Connection $(, $arg: $aty)*
            ) -> $crate::prelude::Result<()> {
                conn.execute($sql, rusqlite::params![$($param),+])?;
                Ok(())
            }
        )+
    };
}
pub(super) use sql_writes;

#[cfg(test)]
#[path = "tests/orm.rs"]
mod tests;
