//! `replica::restore_state` — whether a restored engine may exchange yet
//! (#256, B-4).
//!
//! A restore brings back a database from before some erase, so until the
//! erasure manifest is merged into it the engine must not serve or send what
//! it holds. Two things say so: `<data_dir>/restore.pending`, which a
//! `comemory backup restore` keeps while it swaps, and the `schema_meta` key
//! [`KEY`] (`merging`, or `erasure_unknown` when no established manifest
//! could be merged). The key is a `replica_` key, so `comemory rebuild`
//! carries it. [`admit`] is the one check every replica route and every
//! drain runs before it reads or writes anything; the legacy sync routes run
//! [`refuse_unverified`].

use std::path::PathBuf;

use super::identity::{self, Ensured};
use crate::config::{Config, Paths};
use crate::prelude::*;
use crate::store::{Connection, schema_meta};

/// `schema_meta` key holding the restore state; absent when sync is allowed.
pub const KEY: &str = "replica_restore_state";
/// The file a restore keeps in the data directory while it swaps.
pub const PENDING_FILE: &str = "restore.pending";

/// A restore that has not been verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// A restore is merging the erasure manifest into its staged database.
    Merging,
    /// The restore (or a replaced database) found no established manifest:
    /// what else was erased is unknown until `backup merge-erasures`.
    ErasureUnknown,
}

impl State {
    /// The stored literal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Merging => "merging",
            Self::ErasureUnknown => "erasure_unknown",
        }
    }
}

/// `<data_dir>/restore.pending`.
#[must_use]
pub fn pending_path(paths: &Paths) -> PathBuf {
    paths.data_dir().join(PENDING_FILE)
}

/// The restore state `conn` carries, as stored; `None` when sync is allowed.
///
/// # Errors
/// Propagates SQLite failures.
pub fn read(conn: &Connection) -> Result<Option<String>> {
    Ok(schema_meta::get(conn, KEY)?.filter(|state| !state.is_empty()))
}

/// Record `state` on `conn`.
///
/// # Errors
/// Propagates SQLite failures.
pub fn set(conn: &Connection, state: State) -> Result<()> {
    schema_meta::upsert(conn, KEY, state.as_str())
}

/// Clear the restore state: `conn` may exchange again.
///
/// # Errors
/// Propagates SQLite failures.
pub fn clear(conn: &Connection) -> Result<()> {
    schema_meta::delete(conn, KEY)
}

/// Refuse with [`Error::RestoreUnverified`], naming why, while a restore is
/// still pending in `paths` or a restore state is set on `conn`. Any stored
/// state refuses — one this build does not recognize included.
///
/// # Errors
/// [`Error::RestoreUnverified`]; SQLite and filesystem failures.
pub fn refuse_unverified(paths: &Paths, conn: &Connection) -> Result<()> {
    let why = if pending_path(paths).try_exists()? {
        format!(
            "a restore is still pending in {}; rerun `comemory backup restore <dir> --confirm` \
             to finish it",
            paths.data_dir().display()
        )
    } else if let Some(state) = read(conn)? {
        format!(
            "the restore state is `{state}`: the erasures this engine missed are not merged; \
             run `comemory backup merge-erasures <manifest>`"
        )
    } else {
        return Ok(());
    };
    Err(Error::RestoreUnverified(why))
}

/// Admit one replica read or drain: refuse while a restore is unverified,
/// then check the database against the stream identity
/// ([`identity::ensure`]) — and refuse too when that check just found a
/// replaced database whose erasures it could not verify, since the state it
/// recorded then outlives this request.
///
/// # Errors
/// [`Error::RestoreUnverified`]; everything [`identity::ensure`] returns.
pub fn admit(paths: &Paths, cfg: &Config, conn: &mut Connection) -> Result<Ensured> {
    refuse_unverified(paths, conn)?;
    let ensured = identity::ensure(paths, cfg, conn)?;
    if matches!(ensured, Ensured::ErasureUnknown { .. }) {
        refuse_unverified(paths, conn)?;
    }
    Ok(ensured)
}

#[cfg(test)]
#[path = "tests/restore_state.rs"]
mod tests;
