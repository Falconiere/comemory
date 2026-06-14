//! Shared helpers for `tests/retrieval__code_rerank*.rs`.

use comemory::retrieval::code_route::CodeRoutedHit;
use comemory::retrieval::router::Source;
use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::connection;

/// Open a fresh in-memory-like database in a temp dir and return both
/// the guard and the connection.
pub fn open_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

/// Insert one real `code_symbols` row via the production writer and
/// return its rowid.
pub fn seed(
    conn: &rusqlite::Connection,
    repo: &str,
    path: &str,
    symbol: &str,
    lines: (i64, i64),
    parent_id: Option<i64>,
) -> i64 {
    code_row::insert(
        conn,
        &CodeSymbolRow {
            repo,
            path,
            blob_oid: "oid",
            symbol,
            kind: "function",
            lang: "rust",
            line_start: lines.0,
            line_end: lines.1,
            snippet: "fn body() {}",
            simhash: 0,
            parent_id,
        },
    )
    .expect("insert code symbol")
}

pub fn hit(symbol_id: i64, score: f32) -> CodeRoutedHit {
    CodeRoutedHit {
        symbol_id,
        score,
        source: Source::Lexical,
    }
}
