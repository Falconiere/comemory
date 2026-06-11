//! Shared code-symbol seeding fixture for retrieval-layer tests.
//!
//! Rows go through the production writer (`store::code_row::insert`) — no
//! mock data — so tests exercise the same column defaults (`rank_score`,
//! `access_count`, server-side `indexed_at`) the live `index-code` path
//! produces.

use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::connection;

/// Open a freshly migrated `comemory.db` inside a tempdir.
pub fn open_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

/// Insert one real `code_symbols` row via the production writer and return
/// its rowid. Lines are fixed at `(1, 10)`; never a cAST chunk child.
pub fn seed_symbol(conn: &rusqlite::Connection, repo: &str, path: &str, symbol: &str) -> i64 {
    code_row::insert(
        conn,
        &CodeSymbolRow {
            repo,
            path,
            blob_oid: "oid",
            symbol,
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 10,
            snippet: "fn body() {}",
            simhash: 0,
            parent_id: None,
        },
    )
    .expect("insert code symbol")
}
