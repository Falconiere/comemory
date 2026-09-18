//! Batched `code_symbols` read of the columns a candidate observation needs:
//! the stable `(repo, path, symbol)` identity, the `blob_oid` content version,
//! the line range, and the snippet.
//!
//! [`crate::store::code_row`] writes those rows and
//! [`crate::store::code_feedback`] reads one rowid's identity triple at a time;
//! neither reads many rowids at once, and neither needs the version anchor or
//! the text. `code_rerank::CodeReranked` carries the identity but not
//! `blob_oid` or `snippet`, because fusion drops the passage before a caller
//! sees it.

use std::collections::HashMap;

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;
use toolu_orm::core::value::Value;

use super::orm;
use super::schema_code::{CodeSymbols, code_symbols};
use crate::prelude::*;

/// One `code_symbols` row's identity, content version, location and text.
#[derive(Debug, Clone)]
pub struct CodeText {
    /// Repo label — part of the stable identity.
    pub repo: String,
    /// Path relative to the repo root — part of the stable identity.
    pub path: String,
    /// Qualified symbol name — part of the stable identity.
    pub symbol: String,
    /// Git blob OID of the file at index time: the content version. Never the
    /// rowid, which a re-index recycles.
    pub blob_oid: String,
    /// First line of the symbol, 1-based.
    pub line_start: i64,
    /// Last line of the symbol, 1-based inclusive.
    pub line_end: i64,
    /// The symbol's raw source text.
    pub snippet: String,
}

/// Fetch [`CodeText`] for every id in `ids`, keyed by `code_symbols.id`.
///
/// Ids with no live row are simply absent from the map: a re-index purges and
/// reinserts a touched file's rows, so a rowid that vanished between a search
/// and this read is an ordinary race, not an error. An empty `ids` slice
/// short-circuits rather than building an empty `IN` list. Duplicate ids
/// collapse into one entry.
pub fn fetch(conn: &Connection, ids: &[i64]) -> Result<HashMap<i64, CodeText>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let values: Vec<Value> = ids.iter().copied().map(Value::from).collect();
    let query = CodeSymbols::select()
        .columns_typed(&[
            &code_symbols::id,
            &code_symbols::repo,
            &code_symbols::path,
            &code_symbols::symbol,
            &code_symbols::blob_oid,
            &code_symbols::line_start,
            &code_symbols::line_end,
            &code_symbols::snippet,
        ])
        .filter(code_symbols::id.in_list(&values));
    let rows = orm::query_all(conn, query.to_sql(), |r| {
        Ok((
            r.get::<_, i64>(0)?,
            CodeText {
                repo: r.get(1)?,
                path: r.get(2)?,
                symbol: r.get(3)?,
                blob_oid: r.get(4)?,
                line_start: r.get(5)?,
                line_end: r.get(6)?,
                snippet: r.get(7)?,
            },
        ))
    })?;
    Ok(rows.into_iter().collect())
}

#[cfg(test)]
#[path = "tests/code_text.rs"]
mod tests;
