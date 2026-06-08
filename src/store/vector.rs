//! Insert/query helpers around the sqlite-vec virtual tables.
//!
//! All callers must pass vectors of the configured dim. The dim is
//! locked once at the schema layer and surfaced via `dim_memory()` /
//! `dim_code()`.

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

/// Top-k nearest memories. Optional `repo` filter applied via join.
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
    // `?3` is NULL, so the repo filter is a no-op in that case.
    let sql = "SELECT v.memory_id, v.distance FROM memory_vec v \
                 JOIN memories m ON m.id = v.memory_id \
                WHERE v.embedding MATCH ?1 AND k = ?2 \
                  AND (?3 IS NULL OR m.repo = ?3) \
                  AND m.deleted_at IS NULL \
                ORDER BY v.distance";
    let blob = embed::to_vec_blob(query);
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params![blob, k as i64, repo], |row| {
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

/// Top-k nearest code symbols.
pub fn knn_code(conn: &Connection, query: &[f32], k: usize) -> Result<Vec<CodeHit>> {
    let dim = dim_code(conn)?;
    embed::guard_dim(query, dim)?;
    let mut stmt = conn.prepare(
        "SELECT symbol_id, distance FROM code_vec \
         WHERE embedding MATCH ?1 AND k = ?2 ORDER BY distance",
    )?;
    let rows = stmt
        .query_map(params![embed::to_vec_blob(query), k as i64], |row| {
            Ok(CodeHit {
                symbol_id: row.get(0)?,
                distance: row.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}
