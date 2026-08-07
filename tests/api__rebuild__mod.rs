//! Mirror test for `src/api/rebuild/mod.rs`. `api::rebuild::run` is called
//! directly (no CLI process) against real temp data-dirs seeded by real
//! `comemory save` runs: the markdown replay, the atomic swap's sidecar
//! cleanup, and the leave-the-live-DB-alone error path. The preservation
//! copy is covered in `tests/api__rebuild__copy.rs`; the CLI's own
//! byte-compat coverage stays in `tests/cli__rebuild.rs` /
//! `tests/cli__rebuild_2.rs`, and the HTTP job route in
//! `tests/serve__routes__maint__admin.rs`.

#[path = "common/cli_rebuild_support.rs"]
pub mod support;

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use rusqlite::Connection;
use tempfile::{TempDir, tempdir};

use support::{count, open_db, open_db_with_vec, run_save};

/// Call `api::rebuild::run` over `home`'s data dir with a `Ctx::lazy` — the
/// same shape the HTTP job uses, and the one that proves `rebuild` never
/// needs a caller-owned connection.
fn run_rebuild_api(home: &TempDir) -> comemory::prelude::Result<()> {
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    api::rebuild::run(&mut ctx, api::rebuild::Request {})
}

fn assert_edge(conn: &Connection, rel: &str, dst_kind: &str, dst_id: &str) {
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM edges WHERE src_kind = 'memory' AND rel = ?1 \
               AND dst_kind = ?2 AND dst_id = ?3",
            rusqlite::params![rel, dst_kind, dst_id],
            |r| r.get(0),
        )
        .expect("count edges");
    assert_eq!(n, 1, "expected edge {rel} -> {dst_kind}:{dst_id}");
}

#[test]
fn run_reconstructs_memories_from_markdown() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "body one"]);
    run_save(&home, &["--kind", "note", "body two"]);
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");

    run_rebuild_api(&home).expect("rebuild");

    let conn = open_db(&home);
    assert_eq!(count(&conn, "SELECT count(*) FROM memories"), 2);
}

#[test]
fn run_restores_tags_fts_and_edges() {
    let home = tempdir().expect("tempdir");
    run_save(
        &home,
        &[
            "--kind",
            "decision",
            "--repo",
            "qwick",
            "--author",
            "alice",
            "--tags",
            "db,postgres",
            "see `qwick:src/lib.rs` for the rationale",
        ],
    );
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");

    run_rebuild_api(&home).expect("rebuild");

    let conn = open_db_with_vec(&home);
    assert_eq!(count(&conn, "SELECT count(*) FROM memories"), 1);
    assert_eq!(count(&conn, "SELECT count(*) FROM memory_tags"), 2);
    let fts_hits = count(
        &conn,
        "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'rationale'",
    );
    assert!(fts_hits >= 1, "expected FTS to find 'rationale'");
    assert_edge(&conn, "in_repo", "repo", "qwick");
    assert_edge(&conn, "authored_by", "author", "alice");
    assert_edge(&conn, "tagged", "tag", "db");
    assert_edge(&conn, "tagged", "tag", "postgres");
    assert_edge(&conn, "references_file", "file", "qwick:src/lib.rs");
}

#[test]
fn run_skips_hidden_staging_files() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "kept body"]);
    // A stale staging file in `memories/` must be skipped, not parsed as
    // YAML frontmatter.
    let stale = home.path().join("memories").join(".abc12345.tmp");
    std::fs::write(&stale, "garbage not yaml").expect("write stale");
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");

    run_rebuild_api(&home).expect("rebuild");

    let conn = open_db(&home);
    assert_eq!(count(&conn, "SELECT count(*) FROM memories"), 1);
}

#[test]
fn run_cleans_up_the_tmp_db_and_its_wal_shm_sidecars() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "sidecar cleanup body"]);

    run_rebuild_api(&home).expect("rebuild");

    let live = home.path().join("comemory.db");
    let tmp = home.path().join("comemory.db.rebuild.tmp");
    for path in [&live, &tmp] {
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = path.clone().into_os_string();
            sidecar.push(suffix);
            let sidecar = std::path::PathBuf::from(sidecar);
            assert!(
                !sidecar.exists(),
                "rebuild must not leave a sidecar at {}",
                sidecar.display()
            );
        }
    }
    assert!(!tmp.exists(), "rebuild must not leave the tmp DB behind");
}

#[test]
fn run_leaves_the_live_db_intact_when_the_tmp_path_is_blocked() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "original body"]);
    let original = {
        let conn = open_db(&home);
        count(&conn, "SELECT count(*) FROM memories")
    };

    // SQLite cannot open a directory as a database, so `connection::open`
    // on the tmp path fails well before the rename that would clobber the
    // live DB.
    let tmp_path = home.path().join("comemory.db.rebuild.tmp");
    std::fs::create_dir_all(&tmp_path).expect("create dir at tmp path");

    let err = run_rebuild_api(&home).expect_err("blocked tmp path must error");
    // `connection::open(tmp_path)` calls `rusqlite::Connection::open` on a
    // path that is a directory — SQLite refuses with `SQLITE_CANTOPEN`,
    // rendered by rusqlite's `Display` as "unable to open database file:
    // <path>" and wrapped verbatim into `Error::Sqlite`'s "sqlite: {0}".
    let msg = err.to_string();
    assert!(
        msg.contains("unable to open database file"),
        "expected the real SQLite cannot-open failure, got: {msg}"
    );
    assert!(
        msg.contains(tmp_path.to_string_lossy().as_ref()),
        "expected the blocked tmp path in the error, got: {msg}"
    );

    let conn = open_db(&home);
    assert_eq!(
        count(&conn, "SELECT count(*) FROM memories"),
        original,
        "the original DB must survive a failed rebuild"
    );
}

#[test]
fn run_on_a_fresh_data_dir_with_no_memories_builds_an_empty_mirror() {
    let home = tempdir().expect("tempdir");

    run_rebuild_api(&home).expect("rebuild on a fresh data dir");

    let conn = open_db(&home);
    assert_eq!(count(&conn, "SELECT count(*) FROM memories"), 0);
}
