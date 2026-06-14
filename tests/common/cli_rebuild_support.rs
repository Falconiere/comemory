//! Shared helpers for `tests/cli__rebuild.rs` and `tests/cli__rebuild_2.rs`.

use assert_cmd::Command;
use rusqlite::Connection;
use tempfile::TempDir;

/// Run `comemory save [args…]` under `home`'s data dir.
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
pub fn run_rebuild(home: &TempDir) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["rebuild"])
        .assert()
        .success();
}

/// Open the raw SQLite DB (no extensions).
pub fn open_db(home: &TempDir) -> Connection {
    Connection::open(home.path().join("comemory.db")).expect("open")
}

/// Open the DB with `sqlite-vec` + identifier tokenizer loaded.
pub fn open_db_with_vec(home: &TempDir) -> Connection {
    comemory::store::connection::open(home.path().join("comemory.db")).expect("open with vec0")
}

/// Run a single `count(*)` query.
pub fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count")
}
