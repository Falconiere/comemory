//! The remaining SQL behind `comemory doctor`'s health checks — mirror
//! parity's stored-hash scan, the repo-root inventory, and the live memory
//! count. `checks::run_all` keeps the drift comparison, the git-root
//! existence check, and every `Check`/status assembly (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).

use std::collections::HashMap;

use rusqlite::Connection;

use crate::prelude::*;

/// `(id, content_hash)` for every live `memories` row, as a map keyed by id
/// — one query for the whole corpus rather than one per markdown file (see
/// `checks::mirror_parity`, which diffs this against each file's freshly
/// computed hash).
pub fn live_memory_hashes(conn: &Connection) -> Result<HashMap<String, String>> {
    let mut stmt =
        conn.prepare("SELECT id, content_hash FROM memories WHERE deleted_at IS NULL")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

/// `(repo, root_path)` for every `repo_marker` row with a recorded root,
/// behind `checks::repo_roots`'s on-disk existence check.
pub fn repo_roots(conn: &Connection) -> Result<Vec<(String, String)>> {
    let mut stmt =
        conn.prepare("SELECT repo, root_path FROM repo_marker WHERE root_path IS NOT NULL")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Live (non-soft-deleted) `memories` row count, behind
/// `checks::markdown_db_counts`'s comparison against the on-disk markdown
/// file count. `COUNT(*)` is always an `i64` to the driver; the caller's
/// `u64::try_from(..).unwrap_or(0)` fallback is unreachable since this is
/// never negative.
pub fn live_memory_count(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE deleted_at IS NULL",
        [],
        |r| r.get(0),
    )
    .map_err(Error::from)
}

#[cfg(test)]
#[path = "tests/doctor_probes.rs"]
mod tests;
