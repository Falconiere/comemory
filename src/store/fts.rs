//! FTS5 wrappers for memory and code lexical search.

use rusqlite::{params, Connection};

use crate::prelude::*;

/// FTS5 hit for the memory table; lower `score` (BM25) = better match.
pub struct MemoryFtsHit {
    /// Identifier of the matched memory row.
    pub memory_id: String,
    /// BM25 relevance score; lower is better.
    pub score: f32,
}

/// FTS5 hit for the code table; lower `score` (BM25) = better match.
pub struct CodeFtsHit {
    /// Identifier of the matched `code_symbols` row.
    pub symbol_id: i64,
    /// BM25 relevance score; lower is better.
    pub score: f32,
}

/// Insert a row into the `memory_fts` virtual table indexing the memory body and tags.
pub fn index_memory(conn: &Connection, memory_id: &str, body: &str, tags_csv: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_fts(memory_id, body, tags) VALUES(?1, ?2, ?3)",
        params![memory_id, body, tags_csv],
    )?;
    Ok(())
}

/// Run a BM25 search over `memory_fts`, skipping soft-deleted memories.
///
/// Optional `repo` filter is applied via the same JOIN that gates on
/// `deleted_at`, so the lexical and vector branches share the same scope
/// when a hybrid query is run with a repo filter. FTS5 MATCH parse errors
/// (malformed user query syntax, e.g. a stray apostrophe or unbalanced
/// quote) are downgraded to an empty result rather than propagated, so a
/// typo in the query string cannot abort the wider retrieval pipeline.
pub fn search_memory(
    conn: &Connection,
    query: &str,
    k: usize,
    repo: Option<&str>,
) -> Result<Vec<MemoryFtsHit>> {
    if query.trim().is_empty() || k == 0 {
        return Ok(Vec::new());
    }
    let sql = match repo {
        Some(_) => {
            "SELECT memory_fts.memory_id, bm25(memory_fts) AS score \
               FROM memory_fts \
               JOIN memories m ON m.id = memory_fts.memory_id \
              WHERE memory_fts MATCH ?1 AND m.deleted_at IS NULL AND m.repo = ?3 \
              ORDER BY score \
              LIMIT ?2"
        }
        None => {
            "SELECT memory_fts.memory_id, bm25(memory_fts) AS score \
               FROM memory_fts \
               JOIN memories m ON m.id = memory_fts.memory_id \
              WHERE memory_fts MATCH ?1 AND m.deleted_at IS NULL \
              ORDER BY score \
              LIMIT ?2"
        }
    };
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) if is_fts5_parse_error(&e) => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let row_fn = |row: &rusqlite::Row<'_>| {
        Ok(MemoryFtsHit {
            memory_id: row.get(0)?,
            score: row.get(1)?,
        })
    };
    let mut out = Vec::new();
    match repo {
        Some(r) => collect_with_fts5_guard(
            stmt.query_map(params![query, k as i64, r], row_fn),
            &mut out,
        )?,
        None => {
            collect_with_fts5_guard(stmt.query_map(params![query, k as i64], row_fn), &mut out)?
        }
    }
    Ok(out)
}

/// Drain a `query_map` result iterator, downgrading any FTS5 MATCH parse
/// error to an empty result and propagating every other SQLite error. Used
/// by both [`search_memory`] branches so the repo-filtered and unfiltered
/// SQL paths share the same parse-error handling.
fn collect_with_fts5_guard<R, F>(
    iter: rusqlite::Result<rusqlite::MappedRows<'_, F>>,
    out: &mut Vec<R>,
) -> Result<()>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<R>,
{
    let mapped = match iter {
        Ok(it) => it,
        Err(e) if is_fts5_parse_error(&e) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for row in mapped {
        match row {
            Ok(hit) => out.push(hit),
            Err(e) if is_fts5_parse_error(&e) => {
                out.clear();
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Best-effort classification for FTS5 MATCH-expression parse errors.
/// FTS5 surfaces these as `SQLITE_ERROR` with a `fts5:` prefix or a
/// `syntax error near "<token>"` substring on older builds. Used by
/// [`search_memory`] / [`search_code`] to downgrade a user-typed
/// malformed query to an empty result rather than aborting the wider
/// retrieval pipeline.
fn is_fts5_parse_error(e: &rusqlite::Error) -> bool {
    let s = e.to_string().to_lowercase();
    s.starts_with("fts5:") || s.contains("fts5") || s.contains("syntax error")
}

/// Insert a row into the `code_fts` virtual table for a code symbol.
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

/// Run a BM25 search over `code_fts` and return the top-`k` symbol hits.
/// FTS5 MATCH parse errors are downgraded to an empty result for the same
/// reason as [`search_memory`] — a typo in the user query should not abort
/// the wider retrieval pipeline.
pub fn search_code(conn: &Connection, query: &str, k: usize) -> Result<Vec<CodeFtsHit>> {
    if query.trim().is_empty() || k == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = match conn.prepare(
        "SELECT symbol_id, bm25(code_fts) AS score \
           FROM code_fts \
          WHERE code_fts MATCH ?1 \
          ORDER BY score \
          LIMIT ?2",
    ) {
        Ok(s) => s,
        Err(e) if is_fts5_parse_error(&e) => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mapped = match stmt.query_map(params![query, k as i64], |row| {
        Ok(CodeFtsHit {
            symbol_id: row.get(0)?,
            score: row.get(1)?,
        })
    }) {
        Ok(it) => it,
        Err(e) if is_fts5_parse_error(&e) => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    for row in mapped {
        match row {
            Ok(hit) => out.push(hit),
            Err(e) if is_fts5_parse_error(&e) => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(out)
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
