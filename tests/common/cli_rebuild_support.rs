#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Shared helpers for `tests/cli__rebuild.rs` and `tests/cli__rebuild_2.rs`.

use assert_cmd::Command;
use rusqlite::Connection;
use tempfile::TempDir;

/// Run `comemory save [args…]` under `home`'s data dir.
///
/// # Panics
///
/// Panics if the `comemory` binary is missing or the save fails.
pub fn run_save(home: &TempDir, args: &[&str]) {
    let mut cmd = Command::cargo_bin("comemory").expect("bin");
    cmd.env("COMEMORY_DATA_DIR", home.path());
    cmd.arg("save");
    for a in args {
        cmd.arg(a);
    }
    cmd.assert().success();
}

/// Run `comemory rebuild` under `home`'s data dir.
///
/// # Panics
///
/// Panics if the `comemory` binary is missing or the rebuild fails.
pub fn run_rebuild(home: &TempDir) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["rebuild"])
        .assert()
        .success();
}

/// Open the raw SQLite DB (no extensions).
///
/// # Panics
///
/// Panics if the database cannot be opened.
pub fn open_db(home: &TempDir) -> Connection {
    Connection::open(home.path().join("comemory.db")).expect("open")
}

/// Open the DB with `sqlite-vec` + identifier tokenizer loaded.
///
/// # Panics
///
/// Panics if the database cannot be opened or an extension fails to load.
pub fn open_db_with_vec(home: &TempDir) -> Connection {
    comemory::store::connection::open(home.path().join("comemory.db")).expect("open with vec0")
}

/// Run a single `count(*)` query.
///
/// # Panics
///
/// Panics if the query fails.
pub fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count")
}
