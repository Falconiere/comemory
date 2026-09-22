//! `code_generation` row lifecycle — one row per generation of one repo's
//! index, local or pulled.
//!
//! [`activate`] takes the caller's transaction on purpose: an acceptance has
//! to flip the generation and write its journal position together, or a peer
//! is told about a position whose projection is not there.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;
use toolu_orm::query::insert::OnConflict;

use super::orm;
use super::replica_journal::ReplicaOrigin;
use super::schema_code_generation::{CodeGeneration, code_generation as col};
use crate::prelude::*;

/// Where a generation is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Recorded but not yet complete; invisible to every reader.
    Staged,
    /// The repo's current generation.
    Active,
    /// Replaced by a later activation.
    Superseded,
}

impl State {
    /// The stored token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Active => "active",
            Self::Superseded => "superseded",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "staged" => Ok(Self::Staged),
            "active" => Ok(Self::Active),
            "superseded" => Ok(Self::Superseded),
            other => Err(Error::Other(format!(
                "code_generation.state holds an unknown value: {other}"
            ))),
        }
    }
}

/// One generation of a repo's index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generation {
    /// Canonical repo label.
    pub repo: String,
    /// 32-hex digest of the canonical payload.
    pub generation_id: String,
    /// The generation this was planned against; `None` for a repo's first.
    pub parent_id: Option<String>,
    /// HEAD commit the index was built at.
    pub head: String,
    /// Co-change cursor at build time, when the repo has one.
    pub mined_commit: Option<String>,
    /// Whether this machine built it or a peer sent it.
    pub origin: ReplicaOrigin,
    /// Where it is in its life.
    pub state: State,
    /// How many files the manifest names.
    pub file_count: i64,
    /// 64-hex digest over the manifest, symbols and edges.
    pub manifest_digest: String,
}

/// Record `generation`, replacing any earlier row for the same
/// `(repo, generation_id)`.
///
/// A re-recorded generation is the same generation — the id is the digest of
/// its contents — so this is an upsert rather than a conflict.
///
/// # Errors
/// Propagates SQLite failures.
pub fn record(conn: &Connection, generation: &Generation, at: &str) -> Result<()> {
    orm::execute(
        conn,
        CodeGeneration::insert()
            .set(&col::repo, generation.repo.as_str())
            .set(&col::generation_id, generation.generation_id.as_str())
            .set(&col::parent_id, generation.parent_id.as_deref())
            .set(&col::head, generation.head.as_str())
            .set(&col::mined_commit, generation.mined_commit.as_deref())
            .set(&col::origin, generation.origin.as_str())
            .set(&col::state, generation.state.as_str())
            .set(&col::file_count, generation.file_count)
            .set(&col::manifest_digest, generation.manifest_digest.as_str())
            .set(&col::created_at, at)
            .on_conflict(
                OnConflict::column(&col::repo)
                    .and_column(&col::generation_id)
                    .set(&col::parent_id, generation.parent_id.as_deref())
                    .set(&col::head, generation.head.as_str())
                    .set(&col::mined_commit, generation.mined_commit.as_deref())
                    .set(&col::origin, generation.origin.as_str())
                    .set(&col::state, generation.state.as_str())
                    .set(&col::file_count, generation.file_count)
                    .set(&col::manifest_digest, generation.manifest_digest.as_str()),
            )
            .to_sql(),
    )?;
    Ok(())
}

/// Make `generation_id` the repo's active generation, superseding the one it
/// replaces — both in the caller's transaction.
///
/// A generation whose recorded parent is not the repo's current active one is
/// a stale plan; merging it would union two heads, so it is refused and the
/// caller replans. Activating the already-active generation is a no-op, which
/// is what makes a replayed acceptance cheap rather than an error.
///
/// # Errors
/// SQLite failures; [`Error::Conflict`] for a stale parent, [`Error::NotFound`]
/// when the generation was never recorded.
pub fn activate(tx: &Connection, repo: &str, generation_id: &str, at: &str) -> Result<()> {
    let Some(target) = by_id(tx, repo, generation_id)? else {
        return Err(Error::NotFound(format!(
            "code generation {generation_id} for {repo}"
        )));
    };
    let current = active(tx, repo)?;
    if current.as_ref().map(|g| g.generation_id.as_str()) == Some(generation_id) {
        return Ok(());
    }
    let current_id = current.as_ref().map(|g| g.generation_id.as_str());
    if target.parent_id.as_deref() != current_id {
        return Err(Error::Conflict(format!(
            "code generation {generation_id} was planned against {:?}, but {repo} is at {current_id:?}",
            target.parent_id
        )));
    }
    if let Some(current) = current.as_ref() {
        set_state(tx, repo, &current.generation_id, State::Superseded, None)?;
    }
    set_state(tx, repo, generation_id, State::Active, Some(at))
}

/// The repo's active generation, if it has one.
///
/// # Errors
/// Propagates SQLite failures and an unrecognized stored `state`/`origin`.
pub fn active(conn: &Connection, repo: &str) -> Result<Option<Generation>> {
    one(conn, |select| {
        select
            .filter(col::repo.eq(repo))
            .filter(col::state.eq(State::Active.as_str()))
    })
}

/// The repo's active generation, but only when this machine built it.
///
/// Upload selection reads this: a projection a peer sent is never offered
/// back, which is how a replication loop is prevented rather than detected.
///
/// # Errors
/// Propagates SQLite failures and an unrecognized stored value.
pub fn active_local(conn: &Connection, repo: &str) -> Result<Option<Generation>> {
    Ok(active(conn, repo)?.filter(|g| g.origin == ReplicaOrigin::Local))
}

/// One generation by id, whatever its state.
///
/// # Errors
/// Propagates SQLite failures and an unrecognized stored value.
pub fn by_id(conn: &Connection, repo: &str, generation_id: &str) -> Result<Option<Generation>> {
    one(conn, |select| {
        select
            .filter(col::repo.eq(repo))
            .filter(col::generation_id.eq(generation_id))
    })
}

/// Every generation this machine holds for `repo`, newest first.
///
/// # Errors
/// Propagates SQLite failures and an unrecognized stored value.
pub fn all(conn: &Connection, repo: &str) -> Result<Vec<Generation>> {
    rows(
        conn,
        CodeGeneration::select()
            .columns_typed(&COLUMNS)
            .filter(col::repo.eq(repo))
            .order_by(col::created_at.desc())
            .to_sql(),
    )
}

/// Stamp one row's state, and its activation time when it is becoming active.
fn set_state(
    tx: &Connection,
    repo: &str,
    generation_id: &str,
    state: State,
    at: Option<&str>,
) -> Result<()> {
    orm::execute(
        tx,
        CodeGeneration::update()
            .set(&col::state, state.as_str())
            .set(&col::activated_at, at)
            .filter(col::repo.eq(repo))
            .filter(col::generation_id.eq(generation_id))
            .to_sql(),
    )?;
    Ok(())
}

/// Every projected column, in the order [`row`] reads them.
const COLUMNS: [&dyn toolu_orm::core::query_column::ColumnRef; 9] = [
    &col::repo,
    &col::generation_id,
    &col::parent_id,
    &col::head,
    &col::mined_commit,
    &col::origin,
    &col::state,
    &col::file_count,
    &col::manifest_digest,
];

/// Run a single-row query built by narrowing the standard projection.
fn one<F>(conn: &Connection, narrow: F) -> Result<Option<Generation>>
where
    F: FnOnce(
        toolu_orm::query::select::SelectBuilder,
    ) -> toolu_orm::query::select::SelectBuilder,
{
    let select = narrow(CodeGeneration::select().columns_typed(&COLUMNS));
    Ok(rows(conn, select.to_sql())?.into_iter().next())
}

/// Decode a projected result set.
fn rows(conn: &Connection, sql: (String, Vec<toolu_orm::core::value::Value>)) -> Result<Vec<Generation>> {
    orm::query_all(conn, sql, row)?
        .into_iter()
        .map(|(generation, origin, state)| {
            Ok(Generation {
                origin: ReplicaOrigin::parse(&origin)?,
                state: State::parse(&state)?,
                ..generation
            })
        })
        .collect()
}

/// One stored row, with the two enum columns left as raw tokens.
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(Generation, String, String)> {
    Ok((
        Generation {
            repo: r.get(0)?,
            generation_id: r.get(1)?,
            parent_id: r.get(2)?,
            head: r.get(3)?,
            mined_commit: r.get(4)?,
            origin: ReplicaOrigin::Local,
            state: State::Staged,
            file_count: r.get(7)?,
            manifest_digest: r.get(8)?,
        },
        r.get(5)?,
        r.get(6)?,
    ))
}

#[cfg(test)]
#[path = "tests/code_generation.rs"]
mod tests;
