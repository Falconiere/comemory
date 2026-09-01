#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/rebuild/copy.rs`. Every function in that file is
//! private to `api::rebuild`, so the preservation copy is exercised through
//! the public `api::rebuild::run` entry point: seed a real DB with a real
//! ingested code symbol (`comemory ingest-code`) plus real learning-loop
//! rows, rebuild, and assert every SQLite-only table survived the swap.

#[path = "common/cli_rebuild_support.rs"]
pub mod support;
#[path = "common/vectors.rs"]
mod vectors;

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use rusqlite::Connection;
use tempfile::{TempDir, tempdir};

use support::{count, open_db, open_db_with_vec, run_save};

/// Call `api::rebuild::run` over `home`'s data dir with a `Ctx::lazy`.
fn run_rebuild_api(home: &TempDir) {
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    api::rebuild::run(&mut ctx, api::rebuild::Request {}).expect("rebuild");
}

/// Ingest one real code symbol row (with a 768-d embedding) through the real
/// `comemory ingest-code` binary, so `code_symbols` / `code_fts` /
/// `code_vec` / `indexed_files` all carry genuine rows.
fn ingest_code_symbol(home: &TempDir) {
    let row = serde_json::json!({
        "repo": "myrepo",
        "path": "src/lib.rs",
        "blob_oid": "aaaa000000000000000000000000000000000000",
        "symbol": "preserved_fn",
        "kind": "function",
        "lang": "rust",
        "line_start": 1_u32,
        "line_end": 2_u32,
        "snippet": "fn preserved_fn() {}",
        "simhash": 0_i64,
        "embedding": vectors::vector("rebuild-code-seed", 768),
    });
    assert_cmd::Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["ingest-code"])
        .write_stdin(format!("{row}\n"))
        .assert()
        .success();
}

/// Write real learning-loop rows the markdown replay can never reconstruct:
/// an aggregated `feedback` counter, a mined `query_expansions` mapping, and
/// a `co_changed` code-graph edge.
fn seed_learning_state(home: &TempDir, memory_id: &str) {
    let conn = open_db_with_vec(home);
    conn.execute(
        "INSERT INTO feedback(memory_id, used_count, irrelevant_count, last_used) \
         VALUES (?1, 7, 1, '2026-01-01T00:00:00.000000000Z')",
        [memory_id],
    )
    .expect("insert feedback");
    conn.execute(
        "INSERT INTO query_expansions(term, expansion, support, last_mined) \
         VALUES ('pg', 'postgres', 4, '2026-01-01T00:00:00.000000000Z')",
        [],
    )
    .expect("insert query_expansions");
    conn.execute(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, weight, created_at) \
         VALUES ('file', 'myrepo:src/lib.rs', 'file', 'myrepo:src/main.rs', 'co_changed', 3, \
                 '2026-01-01T00:00:00.000000000Z')",
        [],
    )
    .expect("insert co_changed edge");
}

/// The single live memory's id, read out of the mirror.
fn only_memory_id(conn: &Connection) -> String {
    conn.query_row("SELECT id FROM memories LIMIT 1", [], |r| r.get(0))
        .expect("one memory row")
}

#[test]
fn rebuild_preserves_the_code_index_tables() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "code index preservation body"]);
    ingest_code_symbol(&home);
    {
        let conn = open_db(&home);
        assert_eq!(count(&conn, "SELECT count(*) FROM code_symbols"), 1);
    }

    run_rebuild_api(&home);

    let conn = open_db_with_vec(&home);
    assert_eq!(
        count(&conn, "SELECT count(*) FROM memories"),
        1,
        "rebuild must restore the memory row"
    );
    assert_eq!(
        count(&conn, "SELECT count(*) FROM code_symbols"),
        1,
        "rebuild must preserve code_symbols rows"
    );
    assert_eq!(
        count(&conn, "SELECT count(*) FROM indexed_files"),
        1,
        "rebuild must preserve indexed_files rows"
    );
    assert_eq!(
        count(&conn, "SELECT count(*) FROM code_vec"),
        1,
        "the vec0 cross-DB INSERT-SELECT must carry embeddings over"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM code_fts WHERE code_fts MATCH 'preserved_fn'"
        ),
        1,
        "rebuild must preserve code_fts rows"
    );
}

#[test]
fn rebuild_preserves_learning_counters_expansions_and_mined_edges() {
    let home = tempdir().expect("tempdir");
    run_save(
        &home,
        &["--kind", "note", "learning state preservation body"],
    );
    ingest_code_symbol(&home);
    let memory_id = {
        let conn = open_db(&home);
        only_memory_id(&conn)
    };
    seed_learning_state(&home, &memory_id);

    run_rebuild_api(&home);

    let conn = open_db_with_vec(&home);
    let used: i64 = conn
        .query_row(
            "SELECT used_count FROM feedback WHERE memory_id = ?1",
            [&memory_id],
            |r| r.get(0),
        )
        .expect("feedback row must survive the rebuild");
    assert_eq!(used, 7, "the Beta feedback counter must be carried over");
    let expansion: String = conn
        .query_row(
            "SELECT expansion FROM query_expansions WHERE term = 'pg'",
            [],
            |r| r.get(0),
        )
        .expect("mined expansion must survive the rebuild");
    assert_eq!(expansion, "postgres");
    let weight: i64 = conn
        .query_row(
            "SELECT weight FROM edges WHERE rel = 'co_changed' AND src_id = 'myrepo:src/lib.rs'",
            [],
            |r| r.get(0),
        )
        .expect("mined co_changed edge must survive the rebuild");
    assert_eq!(weight, 3, "mined edge weights must be carried over");
}

#[test]
fn rebuild_with_no_pre_existing_db_skips_the_copy_entirely() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "no old db body"]);
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");
    let _ = std::fs::remove_file(home.path().join("comemory.db-wal"));
    let _ = std::fs::remove_file(home.path().join("comemory.db-shm"));

    run_rebuild_api(&home);

    let conn = open_db(&home);
    assert_eq!(count(&conn, "SELECT count(*) FROM memories"), 1);
    assert_eq!(
        count(&conn, "SELECT count(*) FROM code_symbols"),
        0,
        "nothing to preserve when the old DB is gone"
    );
}

/// v15 preservation: the `index_runs` history, an `eval_runs` row's
/// `discarded` flag, and a `repo_marker`'s `root_path` + `archived` all
/// survive a rebuild — none of them is reconstructable from markdown, and a
/// rebuild that dropped them would erase the run history, re-offer every
/// discarded proposal, and forget where (and whether) a repo is indexed.
#[test]
fn rebuild_preserves_run_history_and_the_v15_flags() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "history preservation body"]);
    {
        let conn = open_db_with_vec(&home);
        conn.execute(
            "INSERT INTO index_runs(id, repo, root_path, mode, started_at, finished_at, \
                                    duration_ms, files_indexed, symbols, outcome, error) \
             VALUES ('0f1e2d3c4b5a6978', 'myrepo', '/tmp/myrepo', 'full', \
                     '2026-09-01T10:00:00.000000000Z', '2026-09-01T10:00:02.000000000Z', \
                     2000, 12, 340, 'ok', NULL)",
            [],
        )
        .expect("insert index_runs");
        conn.execute(
            "INSERT INTO eval_runs(id, kind, at, golden_pairs, k, recall, mrr, knobs, \
                                   applied, discarded) \
             VALUES ('a1b2c3d4e5f60718', 'tune', '2026-08-31T10:00:00.000000000Z', 96, 10, \
                     0.78, 0.61, '{\"rrf_k\":60.0}', 0, 1)",
            [],
        )
        .expect("insert eval_runs");
        conn.execute(
            "INSERT INTO repo_marker(repo, last_head, last_indexed_at, root_path, archived) \
             VALUES ('myrepo', 'abc123', '2026-09-01T10:00:00.000000000Z', '/tmp/myrepo', 1)",
            [],
        )
        .expect("insert repo_marker");
    }

    run_rebuild_api(&home);

    let conn = open_db_with_vec(&home);
    let (files, outcome): (i64, String) = conn
        .query_row(
            "SELECT files_indexed, outcome FROM index_runs WHERE id = '0f1e2d3c4b5a6978'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("index_runs row must survive the rebuild");
    assert_eq!((files, outcome.as_str()), (12, "ok"));
    let discarded: i64 = conn
        .query_row(
            "SELECT discarded FROM eval_runs WHERE id = 'a1b2c3d4e5f60718'",
            [],
            |r| r.get(0),
        )
        .expect("eval_runs row must survive the rebuild");
    assert_eq!(discarded, 1, "a discarded proposal must stay discarded");
    let (root, archived): (String, i64) = conn
        .query_row(
            "SELECT root_path, archived FROM repo_marker WHERE repo = 'myrepo'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("repo_marker row must survive the rebuild");
    assert_eq!(root, "/tmp/myrepo", "root_path must be carried over");
    assert_eq!(archived, 1, "archived must be carried over");
}
