//! FTS5 wrappers for memory and code lexical search.

use rusqlite::{params, Connection};

use crate::prelude::*;

pub struct MemoryFtsHit {
    pub memory_id: String,
    pub score: f32,
}

pub struct CodeFtsHit {
    pub symbol_id: i64,
    pub score: f32,
}

pub fn index_memory(conn: &Connection, memory_id: &str, body: &str, tags_csv: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_fts(memory_id, body, tags) VALUES(?1, ?2, ?3)",
        params![memory_id, body, tags_csv],
    )?;
    Ok(())
}

pub fn search_memory(conn: &Connection, query: &str, k: usize) -> Result<Vec<MemoryFtsHit>> {
    let mut stmt = conn.prepare(
        "SELECT memory_fts.memory_id, bm25(memory_fts) AS score \
           FROM memory_fts \
           JOIN memories m ON m.id = memory_fts.memory_id \
          WHERE memory_fts MATCH ?1 AND m.deleted_at IS NULL \
          ORDER BY score \
          LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![query, k as i64], |row| {
            Ok(MemoryFtsHit {
                memory_id: row.get(0)?,
                score: row.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn index_code(
    conn: &Connection,
    symbol_id: i64,
    symbol: &str,
    snippet: &str,
    path_tokens: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO code_fts(symbol_id, symbol, snippet, path_tokens) \
         VALUES(?1, ?2, ?3, ?4)",
        params![symbol_id, symbol, snippet, path_tokens],
    )?;
    Ok(())
}

pub fn search_code(conn: &Connection, query: &str, k: usize) -> Result<Vec<CodeFtsHit>> {
    let mut stmt = conn.prepare(
        "SELECT symbol_id, bm25(code_fts) AS score \
           FROM code_fts \
          WHERE code_fts MATCH ?1 \
          ORDER BY score \
          LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![query, k as i64], |row| {
            Ok(CodeFtsHit {
                symbol_id: row.get(0)?,
                score: row.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Split a path into BM25-friendly tokens: lowercase, alnum runs.
/// Used by index-code to populate `code_fts.path_tokens`.
pub fn path_to_tokens(path: &str) -> String {
    path.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
