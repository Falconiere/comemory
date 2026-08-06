//! Shared command core between `cli::` and `serve::routes::`:
//! `api::<cmd>::run(&mut Ctx, Request)` holds each subcommand's logic, so
//! neither surface duplicates it (precedent:
//! `retrieval::code_search::search_code_hits`).
//!
//! [`Ctx`] bundles [`Paths`] + [`Config`] with a connection that is either
//! borrowed or opened lazily on first use, so conn-free commands (`doctor`,
//! `rebuild`, `ast`, `install-hooks`, `completions`) never touch the DB.

use once_cell::sync::OnceCell;
use rusqlite::Connection;

use crate::config::{Config, Paths};
use crate::prelude::*;

/// Borrowed execution context passed to every `api::<cmd>::run`.
///
/// Construct via [`Ctx::borrowed`] or [`Ctx::lazy`]; reach the connection
/// through [`Ctx::conn`].
pub struct Ctx<'a> {
    /// Resolved data-directory layout (`memories/`, `comemory.db`, …).
    pub paths: &'a Paths,
    /// Layered configuration (defaults → file → env).
    pub cfg: &'a Config,
    db: DbSource<'a>,
}

/// Where [`Ctx::conn`] gets its [`Connection`] from.
enum DbSource<'a> {
    /// A connection the caller already owns and keeps open for the call.
    Borrowed(&'a mut Connection),
    /// Opened via [`crate::store::connection::open`] on first [`Ctx::conn`]
    /// call, then reused for the rest of this `Ctx`'s life.
    Lazy(OnceCell<Connection>),
}

impl<'a> Ctx<'a> {
    /// Build a `Ctx` around a connection the caller already owns (the CLI
    /// request path, or the server's shared per-request connection).
    pub fn borrowed(paths: &'a Paths, cfg: &'a Config, conn: &'a mut Connection) -> Self {
        Self {
            paths,
            cfg,
            db: DbSource::Borrowed(conn),
        }
    }

    /// Build a `Ctx` that opens `comemory.db` only when [`Ctx::conn`] is
    /// first called — used by job workers (their own dedicated connection)
    /// and conn-free commands (never opened at all).
    pub fn lazy(paths: &'a Paths, cfg: &'a Config) -> Self {
        Self {
            paths,
            cfg,
            db: DbSource::Lazy(OnceCell::new()),
        }
    }

    /// The SQLite connection, opening it lazily on first use when this
    /// `Ctx` was built via [`Ctx::lazy`].
    pub fn conn(&mut self) -> Result<&mut Connection> {
        match &mut self.db {
            DbSource::Borrowed(conn) => Ok(conn),
            DbSource::Lazy(cell) => {
                cell.get_or_try_init(|| crate::store::connection::open(self.paths.db_path()))?;
                cell.get_mut()
                    .ok_or_else(|| Error::Other("lazy connection missing after init".into()))
            }
        }
    }
}
