//! Persist memory activation and materialized graph-rank signals.

use super::{
    orm,
    schema_memory::{Memories, memories},
};
use crate::prelude::*;
use rusqlite::Connection;
use toolu_orm::core::{query_column::CommonOps, value::Value};

/// Write one `rank_score` per memory id, positionally aligned with `scores`.
/// See [`crate::domains::graph::memory_rank::materialize_memory_rank`].
pub(crate) fn update_rank_scores(conn: &Connection, ids: &[String], scores: &[f64]) -> Result<()> {
    for (id, score) in ids.iter().zip(scores) {
        orm::execute(
            conn,
            Memories::update()
                .set(&memories::rank_score, *score)
                .filter(memories::id.eq(id.as_str()))
                .to_sql(),
        )?;
    }
    Ok(())
}

/// Bump `access_count`/`last_accessed` for one chunk of memory ids in a
/// single `UPDATE ... WHERE id IN (...)`. Caller chunks `ids` to stay under
/// SQLite's bound-parameter limit. See
/// [`crate::domains::graph::coactivate::bump_activation`].
pub(crate) fn bump_access(conn: &Connection, ids: &[String], at: &str) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let query = Memories::update()
        .set_expr(&memories::access_count, "access_count + 1")
        .set(&memories::last_accessed, at)
        .filter(
            memories::id.in_list(
                &ids.iter()
                    .map(|id| Value::from(id.as_str()))
                    .collect::<Vec<_>>(),
            ),
        );
    orm::execute(conn, query.to_sql())?;
    Ok(())
}
