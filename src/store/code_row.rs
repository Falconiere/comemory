//! Shared SQLite-mirror writer for a single code-symbol row.
//!
//! Both `cli::index_code` (tree-sitter walk → INSERT) and `cli::ingest_code`
//! (pre-embedded JSONL → INSERT) materialise rows into the same v0.2
//! `code_symbols` table with byte-identical column lists. Centralising the
//! `INSERT ... RETURNING id` SQL here means the two paths cannot drift on
//! column order, timestamp expression, or `RETURNING` clause.
//!
//! The caller decides whether to also write a `code_vec` row — vectors are
//! BYO so `index_code` skips them while `ingest_code` always supplies one.

use rusqlite::Connection;

use crate::prelude::*;

/// Owned column payload for one `code_symbols` row insert.
///
/// Borrowed-reference fields (rather than owned `String`s) keep call sites
/// from cloning when they already hold borrowed data; the helper passes them
/// straight through to `rusqlite::params!`.
pub struct CodeSymbolRow<'a> {
    pub repo: &'a str,
    pub path: &'a str,
    pub blob_oid: &'a str,
    pub symbol: &'a str,
    pub kind: &'a str,
    pub lang: &'a str,
    pub line_start: i64,
    pub line_end: i64,
    pub snippet: &'a str,
    pub simhash: i64,
}

/// Insert one `code_symbols` row and return its newly-assigned primary key.
/// The `indexed_at` column is stamped server-side via `strftime` so callers
/// don't need to format the timestamp themselves.
pub fn insert(conn: &Connection, row: &CodeSymbolRow<'_>) -> Result<i64> {
    let sid = conn.query_row(
        "INSERT INTO code_symbols(\
             repo, path, blob_oid, symbol, kind, lang, \
             line_start, line_end, snippet, simhash, indexed_at) \
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
         RETURNING id",
        rusqlite::params![
            row.repo,
            row.path,
            row.blob_oid,
            row.symbol,
            row.kind,
            row.lang,
            row.line_start,
            row.line_end,
            row.snippet,
            row.simhash,
        ],
        |r| r.get::<_, i64>(0),
    )?;
    Ok(sid)
}
