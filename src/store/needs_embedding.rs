//! `memory_needs_embedding` row CRUD — the memories whose text is stored but
//! whose vector is not, with the reason it is missing.
//!
//! An imported embedding is only usable when it came from the model this
//! engine queries with, at the dimension its `vec0` table was built for.
//! Refusing the vector must never refuse the memory, so the text lands and the
//! id is recorded here for `doctor` to report and `reembed` to drain.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_memory::{MemoryNeedsEmbedding, memory_needs_embedding as col};
use crate::prelude::*;

/// Why a memory holds no vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The operation carried no vector at all.
    Absent,
    /// The vector came from a different model.
    Model,
    /// The vector had a different dimension.
    Dims,
}

impl Reason {
    /// The stored token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Model => "model",
            Self::Dims => "dims",
        }
    }

    /// Parse a stored token.
    fn parse(value: &str) -> Result<Self> {
        match value {
            "absent" => Ok(Self::Absent),
            "model" => Ok(Self::Model),
            "dims" => Ok(Self::Dims),
            other => Err(Error::Other(format!(
                "memory_needs_embedding.reason holds an unknown value: {other}"
            ))),
        }
    }
}

/// One memory still owing a vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The memory id.
    pub memory_id: String,
    /// Why its vector is missing.
    pub reason: Reason,
    /// The model that arrived, when one did.
    pub model: Option<String>,
    /// The dimension that arrived, when one did.
    pub dims: Option<i64>,
}

/// Record that `memory_id` owes a vector, replacing any earlier reason.
///
/// The newest refusal is the one worth reporting: an operator acts on why the
/// vector is missing now, not on why an older import was refused.
///
/// # Errors
/// Propagates SQLite failures.
pub fn record(conn: &Connection, pending: &Pending, at: &str) -> Result<()> {
    let updated = orm::execute(
        conn,
        MemoryNeedsEmbedding::update()
            .set(&col::reason, pending.reason.as_str())
            .set(&col::model, pending.model.as_deref())
            .set(&col::dims, pending.dims)
            .set(&col::recorded_at, at)
            .filter(col::memory_id.eq(pending.memory_id.as_str()))
            .to_sql(),
    )?;
    if updated > 0 {
        return Ok(());
    }
    orm::execute(
        conn,
        MemoryNeedsEmbedding::insert()
            .set(&col::memory_id, pending.memory_id.as_str())
            .set(&col::reason, pending.reason.as_str())
            .set(&col::model, pending.model.as_deref())
            .set(&col::dims, pending.dims)
            .set(&col::recorded_at, at)
            .to_sql(),
    )?;
    Ok(())
}

/// Forget the backlog entry for `memory_id`. A no-op when there is none.
///
/// # Errors
/// Propagates SQLite failures.
pub fn clear(conn: &Connection, memory_id: &str) -> Result<()> {
    orm::execute(
        conn,
        MemoryNeedsEmbedding::delete()
            .filter(col::memory_id.eq(memory_id))
            .to_sql(),
    )?;
    Ok(())
}

/// Every memory still owing a vector, oldest refusal first.
///
/// # Errors
/// Propagates SQLite failures and an unrecognized stored `reason`.
pub fn pending(conn: &Connection) -> Result<Vec<Pending>> {
    let rows: Vec<(String, String, Option<String>, Option<i64>)> = orm::query_all(
        conn,
        MemoryNeedsEmbedding::select()
            .columns_typed(&[&col::memory_id, &col::reason, &col::model, &col::dims])
            .order_by(col::recorded_at.asc())
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    rows.into_iter()
        .map(|(memory_id, reason, model, dims)| {
            Ok(Pending {
                memory_id,
                reason: Reason::parse(&reason)?,
                model,
                dims,
            })
        })
        .collect()
}

/// How many memories still owe a vector.
///
/// # Errors
/// Propagates SQLite failures.
pub fn pending_count(conn: &Connection) -> Result<i64> {
    orm::query_one(conn, MemoryNeedsEmbedding::select().to_count_sql(), |r| {
        r.get(0)
    })
}

#[cfg(test)]
#[path = "tests/needs_embedding.rs"]
mod tests;
