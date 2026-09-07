//! Whether a crate [`Error`] is SQLite's `SQLITE_BUSY` / `SQLITE_LOCKED` —
//! the write lock is held by another connection and the statement can be
//! retried. The only place in the crate that inspects `rusqlite::Error`
//! variants for this predicate, so no caller outside `store::` needs to
//! know the driver's error shape.

use crate::prelude::*;

/// True when `e` wraps `rusqlite`'s `SqliteFailure` with `DatabaseBusy` or
/// `DatabaseLocked` — a transient lock held by another connection, safe to
/// retry with backoff. False for every other [`Error`] variant, including
/// `Error::Sqlite` wrapping a different SQLite failure (e.g.
/// `QueryReturnedNoRows` never reaches here because it is mapped to
/// `Ok(None)` before it becomes an `Error` at all).
pub fn is_locked(e: &Error) -> bool {
    matches!(
        e,
        Error::Sqlite(rusqlite::Error::SqliteFailure(ffi, _))
            if matches!(
                ffi.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

#[cfg(test)]
#[path = "tests/busy.rs"]
mod tests;
