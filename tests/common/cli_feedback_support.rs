//! Shared helpers for `tests/cli__feedback.rs` and
//! `tests/cli__feedback_2.rs`.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
pub fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Open the sandbox `comemory.db` read-only for post-hoc assertions.
pub fn open_db_readonly(home: &TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open_with_flags(
        home.path().join(".comemory").join("comemory.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open db read-only")
}

/// Seed one real `code_symbols` row through the production writer
/// (creating + migrating the sandbox db on first call) and return its
/// rowid. Code feedback resolves ids against live rows now, so the code
/// flags need actual symbols to point at.
pub fn seed_code_symbol(home: &TempDir, repo: &str, path: &str, symbol: &str) -> i64 {
    let data_dir = home.path().join(".comemory");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    let conn = comemory::store::connection::open(data_dir.join("comemory.db")).expect("open db");
    comemory::store::code_row::insert(
        &conn,
        &comemory::store::code_row::CodeSymbolRow {
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

/// Run a `--json` subcommand to success and parse its stdout envelope.
pub fn run_json(home: &TempDir, args: &[&str]) -> Value {
    let mut cmd = bin(home);
    cmd.arg("--json").args(args);
    let assert = cmd.assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(stdout.trim()).expect("parse JSON envelope")
}
