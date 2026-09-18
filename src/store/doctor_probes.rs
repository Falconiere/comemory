//! The remaining SQL behind `comemory doctor`'s health checks — mirror
//! parity's stored-hash scan, the repo-root inventory, and the live memory
//! count. `checks::run_all` keeps the drift comparison, the git-root
//! existence check, and every `Check`/status assembly (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).

use std::collections::HashMap;

use rusqlite::Connection;

use super::{
    orm,
    schema_code::{RepoMarker, repo_marker},
    schema_memory::{Memories, memories},
};
use crate::prelude::*;
use toolu_orm::core::query_column::CommonOps;

/// `(id, content_hash)` for every live `memories` row, as a map keyed by id
/// — one query for the whole corpus rather than one per markdown file (see
/// `checks::mirror_parity`, which diffs this against each file's freshly
/// computed hash).
pub fn live_memory_hashes(conn: &Connection) -> Result<HashMap<String, String>> {
    let query = Memories::select()
        .columns_typed(&[&memories::id, &memories::content_hash])
        .filter(memories::deleted_at.is_null());
    Ok(
        orm::query_all(conn, query.to_sql(), |r| Ok((r.get(0)?, r.get(1)?)))?
            .into_iter()
            .collect(),
    )
}

/// `(repo, root_path)` for every `repo_marker` row with a recorded root,
/// behind `checks::repo_roots`'s on-disk existence check.
pub fn repo_roots(conn: &Connection) -> Result<Vec<(String, String)>> {
    let query = RepoMarker::select()
        .columns_typed(&[&repo_marker::repo, &repo_marker::root_path])
        .filter(repo_marker::root_path.is_not_null());
    orm::query_all(conn, query.to_sql(), |r| Ok((r.get(0)?, r.get(1)?)))
}

/// Live (non-soft-deleted) `memories` row count, behind
/// `checks::markdown_db_counts`'s comparison against the on-disk markdown
/// file count. `COUNT(*)` is always an `i64` to the driver; the caller's
/// `u64::try_from(..).unwrap_or(0)` fallback is unreachable since this is
/// never negative.
pub fn live_memory_count(conn: &Connection) -> Result<i64> {
    let query = Memories::select().filter(memories::deleted_at.is_null());
    orm::query_one(conn, query.to_count_sql(), |r| r.get(0))
}

#[cfg(test)]
#[path = "tests/doctor_probes.rs"]
mod tests;
