//! Task 12: `comemory index-code` now walks a real git repo and uses each
//! file's blob OID as the indexing cursor. The second run over an unchanged
//! repo must short-circuit on the `indexed_files` table so `code_symbols`
//! does not grow.

use assert_cmd::Command;
use tempfile::tempdir;

#[path = "../common/git_setup.rs"]
mod git_setup;

#[test]
fn index_code_writes_symbols_and_skips_unchanged_on_rerun() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = git_setup::build_sample_repo(workspace.path());

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "sample", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();

    let db = home.path().join("comemory.db");
    let conn = rusqlite::Connection::open(&db).expect("open db");
    let initial: i64 = conn
        .query_row("SELECT count(*) FROM code_symbols", [], |r| r.get(0))
        .expect("count code_symbols");
    assert!(initial >= 2, "expected >= 2 symbols, got: {initial}");

    // Second run: nothing changed, `indexed_files` row blocks the re-walk so
    // `code_symbols` must stay at the same row count.
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "sample", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();

    let after: i64 = conn
        .query_row("SELECT count(*) FROM code_symbols", [], |r| r.get(0))
        .expect("count code_symbols");
    assert_eq!(after, initial);
}
