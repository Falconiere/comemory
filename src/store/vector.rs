//! Insert/query helpers around the sqlite-vec virtual tables.
//!
//! All callers must pass vectors of the configured dim. The dim is
//! locked once at the schema layer and surfaced via `dim_memory()` /
//! `dim_code()`.
//!
//! Both `memory_vec` and `code_vec` are created with
//! `distance_metric=cosine` so the KNN distance returned is cosine
//! distance (not L2²). The score formula `score = 1.0 - distance`
//! yields cosine similarity in the range `[-1, 1]`, where `1.0` is
//! identical and `-1.0` is opposite.

use rusqlite::{params, Connection};

use crate::prelude::*;
use crate::store::embed;

/// Result row from a KNN query.
pub struct MemoryHit {
    pub memory_id: String,
    pub distance: f32,
}

/// Read the configured memory vector dim from schema_meta.
pub fn dim_memory(conn: &Connection) -> Result<usize> {
    let v: String = conn.query_row(
        "SELECT value FROM schema_meta WHERE key = 'memory_vector_dim'",
        [],
        |row| row.get(0),
    )?;
    v.parse::<usize>()
        .map_err(|e| Error::Config(format!("memory_vector_dim: {e}")))
}

/// Read the configured code vector dim from schema_meta.
pub fn dim_code(conn: &Connection) -> Result<usize> {
    let v: String = conn.query_row(
        "SELECT value FROM schema_meta WHERE key = 'code_vector_dim'",
        [],
        |row| row.get(0),
    )?;
    v.parse::<usize>()
        .map_err(|e| Error::Config(format!("code_vector_dim: {e}")))
}

/// Insert a memory vector. Dim is validated against schema_meta.
pub fn insert_memory(conn: &Connection, memory_id: &str, vector: &[f32]) -> Result<()> {
    let dim = dim_memory(conn)?;
    embed::guard_dim(vector, dim)?;
    conn.execute(
        "INSERT INTO memory_vec(memory_id, embedding) VALUES(?1, ?2)",
        params![memory_id, embed::to_vec_blob(vector)],
    )?;
    Ok(())
}

/// Oversample factor applied to the vec0 KNN candidate set when a scope
/// filter (memory `repo`, code `repo`/`lang`) is in play. vec0 returns the
/// global nearest-k by cosine distance and the filter runs *after* that,
/// so a corpus spread across multiple repos can drop most of the top-k
/// before the caller ever sees them. Asking for `k * factor` candidates
/// gives the filter room to keep `k` survivors in the common case where
/// the requested scope holds a sizeable fraction of the corpus.
const REPO_FILTER_OVERSAMPLE: usize = 8;

/// Top-k nearest memories. Optional `repo` filter applied via join.
///
/// When `repo` is `Some`, the vec0 candidate set is oversampled by
/// [`REPO_FILTER_OVERSAMPLE`] so the post-filter JOIN against `memories`
/// has enough room to keep `k` survivors. Without oversampling a corpus
/// where the requested repo holds e.g. 20% of the rows would receive only
/// ~`0.2 * k` hits on average, silently undersampling the caller.
pub fn knn_memory(
    conn: &Connection,
    query: &[f32],
    k: usize,
    repo: Option<&str>,
) -> Result<Vec<MemoryHit>> {
    let dim = dim_memory(conn)?;
    embed::guard_dim(query, dim)?;
    // `?3 IS NULL OR m.repo = ?3` lets us bind the optional repo filter as
    // a single SQL string. SQLite short-circuits on the first disjunct when
    // `?3` is NULL, so the repo filter is a no-op in that case. The final
    // `LIMIT ?4` trims the oversampled candidate set back to `k`.
    let sql = "SELECT v.memory_id, v.distance FROM memory_vec v \
                 JOIN memories m ON m.id = v.memory_id \
                WHERE v.embedding MATCH ?1 AND k = ?2 \
                  AND (?3 IS NULL OR m.repo = ?3) \
                  AND m.deleted_at IS NULL \
                ORDER BY v.distance \
                LIMIT ?4";
    let blob = embed::to_vec_blob(query);
    let candidate_k = if repo.is_some() {
        k.saturating_mul(REPO_FILTER_OVERSAMPLE).max(k)
    } else {
        k
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params![blob, candidate_k as i64, repo, k as i64], |row| {
            Ok(MemoryHit {
                memory_id: row.get(0)?,
                distance: row.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Result row from a code KNN query.
pub struct CodeHit {
    pub symbol_id: i64,
    pub distance: f32,
}

/// Insert a code vector. Dim is validated against schema_meta.
pub fn insert_code(conn: &Connection, symbol_id: i64, vector: &[f32]) -> Result<()> {
    let dim = dim_code(conn)?;
    embed::guard_dim(vector, dim)?;
    conn.execute(
        "INSERT INTO code_vec(symbol_id, embedding) VALUES(?1, ?2)",
        params![symbol_id, embed::to_vec_blob(vector)],
    )?;
    Ok(())
}

/// Top-k nearest code symbols, optionally restricted to one `repo`
/// and/or `lang` — the code-side mirror of [`knn_memory`]: the scope
/// predicates JOIN `code_symbols` in the same statement (`?N IS NULL OR
/// c.col = ?N`, a no-op when the filter is absent), and when a filter is
/// in play the vec0 candidate set is oversampled by
/// [`REPO_FILTER_OVERSAMPLE`] for the same reason [`knn_memory`]
/// oversamples: the global nearest-k can live mostly outside the
/// requested scope, and without headroom the join would silently
/// undersample the caller. The final `LIMIT` trims back to `k`.
pub fn knn_code(
    conn: &Connection,
    query: &[f32],
    k: usize,
    repo: Option<&str>,
    lang: Option<&str>,
) -> Result<Vec<CodeHit>> {
    let dim = dim_code(conn)?;
    embed::guard_dim(query, dim)?;
    let sql = "SELECT v.symbol_id, v.distance FROM code_vec v \
                 JOIN code_symbols c ON c.id = v.symbol_id \
                WHERE v.embedding MATCH ?1 AND k = ?2 \
                  AND (?3 IS NULL OR c.repo = ?3) \
                  AND (?4 IS NULL OR c.lang = ?4) \
                ORDER BY v.distance \
                LIMIT ?5";
    let blob = embed::to_vec_blob(query);
    let candidate_k = if repo.is_some() || lang.is_some() {
        k.saturating_mul(REPO_FILTER_OVERSAMPLE).max(k)
    } else {
        k
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(
            params![blob, candidate_k as i64, repo, lang, k as i64],
            |row| {
                Ok(CodeHit {
                    symbol_id: row.get(0)?,
                    distance: row.get(1)?,
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}
