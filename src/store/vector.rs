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

use super::{
    orm,
    schema_code::{CodeVec, code_symbols, code_vec},
    schema_core::{SchemaMeta, schema_meta},
    schema_memory::{MemoryVec, memory_vec},
};
use crate::prelude::*;
use crate::store::CreatedWindow;
use crate::store::embed;
use toolu_orm::core::{
    column::{Integer, Real},
    query_column::{Column, CommonOps, Vec0Ops},
};
use toolu_orm::query::select::SelectBuilder;

/// Result row from a KNN query.
pub struct MemoryHit {
    /// The matched memory's id.
    pub memory_id: String,
    /// Cosine distance to the query vector (lower is closer).
    pub distance: f32,
}

/// Whether the `sqlite-vec` extension is loaded on `conn` — `SELECT
/// vec_version()` succeeds only when the `vec_*` SQL functions and the
/// `vec0` virtual-table module registered on this connection. Behind
/// `comemory doctor`'s "sqlite-vec" check and its `sqlite_vec_loaded` field.
pub fn is_loaded(conn: &Connection) -> bool {
    let query = SelectBuilder::raw().column_expr("vec_version()", "version");
    orm::query_one(conn, query.to_sql(), |r| r.get::<_, String>(0)).is_ok()
}

/// Read the configured memory vector dim from schema_meta.
pub fn dim_memory(conn: &Connection) -> Result<usize> {
    read_dimension(conn, "memory_vector_dim")
}

/// Read the configured code vector dim from schema_meta.
pub fn dim_code(conn: &Connection) -> Result<usize> {
    read_dimension(conn, "code_vector_dim")
}

/// Insert a memory vector. Dim is validated against schema_meta.
pub fn insert_memory(conn: &Connection, memory_id: &str, vector: &[f32]) -> Result<()> {
    write_vector(conn, VectorTarget::Memory(memory_id), vector, false)
}

/// Replace a memory's `memory_vec` row: drop any prior row for `memory_id`,
/// then insert `vector` via [`insert_memory`] (dim validated there, same
/// point it is today). `memory_vec` is a `vec0` virtual table whose primary
/// key does not participate in SQLite's FK cascade, so a bare re-insert on
/// an id that already has a row would leave two rows behind — every
/// re-save (`domains::memories::save`) and re-embed (`maintenance::reembed`) of the same memory
/// must replace, not duplicate.
pub fn replace_memory(conn: &Connection, memory_id: &str, vector: &[f32]) -> Result<()> {
    write_vector(conn, VectorTarget::Memory(memory_id), vector, true)
}

/// Raw `memory_vec.embedding` blob for `memory_id`, or `None` when it has no
/// vector row — behind `comemory sync`'s wire vector encode.
pub fn memory_embedding_blob(conn: &Connection, memory_id: &str) -> Result<Option<Vec<u8>>> {
    let query = MemoryVec::select()
        .columns_typed(&[&memory_vec::embedding])
        .filter(memory_vec::memory_id.eq(memory_id));
    orm::query_optional(conn, query.to_sql(), |r| r.get(0))
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
    /// The matched `code_symbols` rowid.
    pub symbol_id: i64,
    /// Cosine distance to the query vector (lower is closer).
    pub distance: f32,
}

/// Insert a code vector. Dim is validated against schema_meta.
pub fn insert_code(conn: &Connection, symbol_id: i64, vector: &[f32]) -> Result<()> {
    write_vector(conn, VectorTarget::Code(symbol_id), vector, false)
}

/// Replace a code symbol's `code_vec` row — the code-side twin of
/// [`replace_memory`], used by `maintenance::reembed`'s re-vectorize-in-place run.
pub fn replace_code(conn: &Connection, symbol_id: i64, vector: &[f32]) -> Result<()> {
    write_vector(conn, VectorTarget::Code(symbol_id), vector, true)
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
    let blob = embed::to_vec_blob(query);
    let cand = candidate_k(k, repo.is_some() || lang.is_some()) as i64;
    let distance = Column::<Real>::new("code_vec", "distance");
    // Keep k=0 and integer-cast behavior identical to the direct vec0 query.
    let mut select = CodeVec::select()
        .columns_typed(&[&code_vec::symbol_id])
        .column_expr(&distance.qualified(), "distance")
        .join(
            "code_symbols",
            code_symbols::id.equals(&code_vec::symbol_id),
        )
        .filter(
            code_vec::embedding
                .matches(blob)
                .map_err(orm::build_error)?,
        )
        .filter(Column::<Integer>::new("code_vec", "k").eq(cand))
        .order_by(distance.asc())
        .limit(k as i64);
    if let Some(repo) = repo {
        select = select.filter(code_symbols::repo.eq(repo));
    }
    if let Some(lang) = lang {
        select = select.filter(code_symbols::lang.eq(lang));
    }
    orm::query_all(conn, select.to_sql(), |row| {
        Ok(CodeHit {
            symbol_id: row.get(0)?,
            distance: row.get(1)?,
        })
    })
}

/// A vector identity selects one of the two independently sized indexes.
#[derive(Clone, Copy)]
enum VectorTarget<'a> {
    Memory(&'a str),
    Code(i64),
}

/// Share vector validation and encoding, preserving delete-before-validation on replacement.
fn write_vector(
    conn: &Connection,
    target: VectorTarget<'_>,
    vector: &[f32],
    replace: bool,
) -> Result<()> {
    if replace {
        let deletion = match target {
            VectorTarget::Memory(id) => MemoryVec::delete().filter(memory_vec::memory_id.eq(id)),
            VectorTarget::Code(id) => CodeVec::delete().filter(code_vec::symbol_id.eq(id)),
        };
        orm::execute(conn, deletion.to_sql())?;
    }
    let key = match target {
        VectorTarget::Memory(_) => "memory_vector_dim",
        VectorTarget::Code(_) => "code_vector_dim",
    };
    embed::guard_dim(vector, read_dimension(conn, key)?)?;
    let blob = embed::to_vec_blob(vector);
    let insertion = match target {
        VectorTarget::Memory(id) => MemoryVec::insert()
            .set(&memory_vec::memory_id, id)
            .set(&memory_vec::embedding, blob),
        VectorTarget::Code(id) => CodeVec::insert()
            .set(&code_vec::symbol_id, id)
            .set(&code_vec::embedding, blob),
    };
    orm::execute(conn, insertion.to_sql())?;
    Ok(())
}

/// Read and parse one configured index dimension.
fn read_dimension(conn: &Connection, key: &str) -> Result<usize> {
    let query = SchemaMeta::select()
        .columns_typed(&[&schema_meta::value])
        .filter(schema_meta::key.eq(key));
    let v: String = orm::query_one(conn, query.to_sql(), |row| row.get(0))?;
    v.parse::<usize>()
        .map_err(|e| Error::Config(format!("{key}: {e}")))
}

#[cfg(test)]
#[path = "tests/vector.rs"]
mod tests;
