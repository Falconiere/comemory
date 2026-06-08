//! Task 14: `comemory rebuild` drops `comemory.db` and repopulates the
//! SQLite mirror from the on-disk markdown files. Markdown remains the
//! source of truth; the DB is a rebuildable derived cache.

use assert_cmd::Command;
use tempfile::tempdir;

#[test]
fn rebuild_reconstructs_memories_from_markdown() {
    let home = tempdir().expect("tempdir");

    // 1. Save two memories.
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["save", "--kind", "note", "body one"])
        .assert()
        .success();
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["save", "--kind", "note", "body two"])
        .assert()
        .success();

    // 2. Delete the DB; markdown stays.
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");

    // 3. Rebuild.
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["rebuild"])
        .assert()
        .success();

    let conn = rusqlite::Connection::open(home.path().join("comemory.db")).expect("open");
    let cnt: i64 = conn
        .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
        .expect("count");
    assert_eq!(cnt, 2);
}
