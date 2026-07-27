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

use rusqlite::{Connection, params};

use crate::prelude::*;
use crate::store::CreatedWindow;
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
/// filter (memory `repo` / created-date window, code `repo`/`lang`) is in
/// play. vec0 returns the global nearest-k by cosine distance and the
/// filter runs *after* that, so a corpus spread across multiple repos or
/// eras can drop most of the top-k before the caller ever sees them.
/// Asking for `k * factor` candidates gives the filter room to keep `k`
/// survivors in the common case where the requested scope holds a sizeable
/// fraction of the corpus.
const SCOPE_FILTER_OVERSAMPLE: usize = 8;

/// vec0 candidate-set size for one KNN: `k` when nothing filters the
/// result, oversampled by [`SCOPE_FILTER_OVERSAMPLE`] when something does.
fn candidate_k(k: usize, filtered: bool) -> usize {
    if filtered {
        k.saturating_mul(SCOPE_FILTER_OVERSAMPLE).max(k)
    } else {
        k
    }
}

/// Map one `(memory_id, distance)` KNN row.
fn to_memory_hit(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryHit> {
    Ok(MemoryHit {
        memory_id: row.get(0)?,
        distance: row.get(1)?,
    })
}

/// Top-k nearest memories, optionally restricted to one `repo` and/or a
/// created-date `window`, both applied via the `memories` join.
///
/// When any filter is set, the vec0 candidate set is oversampled by
/// [`SCOPE_FILTER_OVERSAMPLE`] so the post-filter JOIN has enough room to
/// keep `k` survivors. Without oversampling a corpus where the requested
/// scope holds e.g. 20% of the rows would receive only ~`0.2 * k` hits on
/// average, silently undersampling the caller.
pub fn knn_memory(
    conn: &Connection,
    query: &[f32],
    k: usize,
    repo: Option<&str>,
    window: CreatedWindow<'_>,
) -> Result<Vec<MemoryHit>> {
    let dim = dim_memory(conn)?;
    embed::guard_dim(query, dim)?;
    // `?3 IS NULL OR m.repo = ?3` (and `?4`/`?5` for the created-date
    // window) binds each optional filter as one SQL string: SQLite
    // short-circuits the disjunct when the parameter is NULL, so an absent
    // filter is a no-op. The window compares through `datetime()` so mixed
    // stored precision cannot invert the order, and `LIMIT ?6` trims the
    // oversampled candidate set back to `k`.
    let sql = "SELECT v.memory_id, v.distance FROM memory_vec v \
                 JOIN memories m ON m.id = v.memory_id \
                WHERE v.embedding MATCH ?1 AND k = ?2 \
                  AND (?3 IS NULL OR m.repo = ?3) \
                  AND (?4 IS NULL OR datetime(m.created_at) >= datetime(?4)) \
                  AND (?5 IS NULL OR datetime(m.created_at) <= datetime(?5)) \
                  AND m.deleted_at IS NULL \
                ORDER BY v.distance \
                LIMIT ?6";
    let blob = embed::to_vec_blob(query);
    let filtered = repo.is_some() || window.since.is_some() || window.cutoff.is_some();
    let cand = candidate_k(k, filtered) as i64;
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(
            params![blob, cand, repo, window.since, window.cutoff, k as i64],
            to_memory_hit,
        )?
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
/// [`SCOPE_FILTER_OVERSAMPLE`] for the same reason [`knn_memory`]
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
    let cand = candidate_k(k, repo.is_some() || lang.is_some()) as i64;
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params![blob, cand, repo, lang, k as i64], |row| {
            Ok(CodeHit {
                symbol_id: row.get(0)?,
                distance: row.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}
