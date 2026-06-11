//! Task 14: `comemory rebuild` drops `comemory.db` and repopulates the
//! SQLite mirror from the on-disk markdown files. Markdown remains the
//! source of truth; the DB is a rebuildable derived cache.
//!
//! These tests exercise more than the row count: tags reappear in
//! `memory_tags`, the FTS index is repopulated, the v0.2 edges
//! (`in_repo` / `authored_by` / `tagged` plus cross-link references
//! parsed from a backticked `repo:path` reference) come back, and hidden
//! staging files in `memories/` are ignored.

use assert_cmd::Command;
use rusqlite::Connection;
use tempfile::{tempdir, TempDir};

fn run_save(home: &TempDir, args: &[&str]) {
    let mut cmd = Command::cargo_bin("comemory").expect("bin");
    cmd.env("COMEMORY_DATA_DIR", home.path());
    cmd.arg("save");
    for a in args {
        cmd.arg(a);
    }
    cmd.assert().success();
}

fn run_rebuild(home: &TempDir) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["rebuild"])
        .assert()
        .success();
}

fn open_db(home: &TempDir) -> Connection {
    Connection::open(home.path().join("comemory.db")).expect("open")
}

/// Like [`open_db`] but routes through `connection::open` so the
/// `sqlite-vec` extension and the `identifier` FTS5 tokenizer are
/// available. Use this when the test needs to SELECT against
/// `code_vec` / `memory_vec` or MATCH against the FTS tables (their v4
/// DDL references `tokenize = 'identifier'`).
fn open_db_with_vec(home: &TempDir) -> Connection {
    comemory::store::connection::open(home.path().join("comemory.db")).expect("open with vec0")
}

#[test]
fn rebuild_reconstructs_memories_from_markdown() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "body one"]);
    run_save(&home, &["--kind", "note", "body two"]);
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");
    run_rebuild(&home);

    let conn = open_db(&home);
    let cnt: i64 = conn
        .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
        .expect("count");
    assert_eq!(cnt, 2);
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count")
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

fn save_rich_memory(home: &TempDir) {
    run_save(
        home,
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
}

fn assert_rebuilt_state(conn: &Connection) {
    assert_eq!(count(conn, "SELECT count(*) FROM memories"), 1);
    assert_eq!(count(conn, "SELECT count(*) FROM memory_tags"), 2);
    let fts_hits: i64 = count(
        conn,
        "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'rationale'",
    );
    assert!(fts_hits >= 1, "expected FTS to find 'rationale'");
    assert_edge(conn, "in_repo", "repo", "qwick");
    assert_edge(conn, "authored_by", "author", "alice");
    assert_edge(conn, "tagged", "tag", "db");
    assert_edge(conn, "tagged", "tag", "postgres");
    assert_edge(conn, "references_file", "file", "qwick:src/lib.rs");
}

#[test]
fn rebuild_restores_tags_fts_and_edges() {
    let home = tempdir().expect("tempdir");
    save_rich_memory(&home);
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");
    run_rebuild(&home);
    let conn = open_db_with_vec(&home);
    assert_rebuilt_state(&conn);
}

#[test]
fn rebuild_skips_hidden_staging_files() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "kept body"]);
    // Drop a stale staging file in `memories/` — rebuild must skip it
    // rather than try to parse it as YAML frontmatter.
    let stale = home.path().join("memories").join(".abc12345.tmp");
    std::fs::write(&stale, "garbage not yaml").expect("write stale");
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");
    run_rebuild(&home);

    let conn = open_db(&home);
    let cnt: i64 = count(&conn, "SELECT count(*) FROM memories");
    assert_eq!(cnt, 1);
}

/// Ingest a code symbol row via `comemory ingest-code` and verify it survives
/// a `comemory rebuild`. The code index tables must be preserved by copying
/// them from the old DB into the newly-built DB.
#[test]
fn rebuild_preserves_code_index() {
    let home = tempdir().expect("tempdir");

    // Save a memory so the DB exists before the ingest.
    run_save(&home, &["--kind", "note", "code index preservation body"]);

    // Ingest one code symbol via ingest-code.
    let embedding = super::vectors::vector("rebuild-code-seed", 768);
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
        "embedding": embedding,
    });
    let payload = format!("{row}\n");
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["ingest-code"])
        .write_stdin(payload)
        .assert()
        .success();

    // Confirm baseline counts.
    {
        let conn = open_db(&home);
        assert_eq!(
            count(&conn, "SELECT count(*) FROM code_symbols"),
            1,
            "pre-rebuild code_symbols"
        );
    }

    // Rebuild: must preserve the code index.
    run_rebuild(&home);

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
    // Regression for PR #3 xhigh review: vec0 cross-DB INSERT-SELECT must
    // actually carry the embedding rows over, not silently drop them.
    assert_eq!(
        count(&conn, "SELECT count(*) FROM code_vec"),
        1,
        "rebuild must preserve code_vec rows"
    );
}

/// Regression for PR #3 review thread: after a successful rebuild, neither
/// the live DB path nor the tmp path should leave orphaned `*-wal` /
/// `*-shm` sidecars in the data dir. SQLite leaves these next to the main
/// file after a WAL-mode close, and the tmp connection's sidecars used to
/// linger forever because the cleanup loop only iterated against `db`.
#[test]
fn rebuild_cleans_up_wal_shm_sidecars() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "sidecar cleanup body"]);

    run_rebuild(&home);

    let data_dir = home.path();
    let live = data_dir.join("comemory.db");
    let tmp = data_dir.join("comemory.db.rebuild.tmp");
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
    assert!(
        !tmp.exists(),
        "rebuild must not leave the tmp DB at {}",
        tmp.display()
    );
}

/// Run a `--json` subcommand against `home` and parse its stdout envelope.
fn run_json(home: &TempDir, args: &[&str]) -> serde_json::Value {
    let mut cmd = Command::cargo_bin("comemory").expect("bin");
    cmd.env("COMEMORY_DATA_DIR", home.path());
    cmd.arg("--json").args(args);
    let assert = cmd.assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(stdout.trim()).expect("parse JSON envelope")
}

/// Learning-loop state (`feedback` counters, `feedback_events` provenance,
/// `retrieval_log` telemetry, mined `query_expansions`) lives only in SQLite
/// — there is no markdown to rebuild it from — so `comemory rebuild` must
/// carry all four tables across, exactly like the code index. Regression for
/// the M2 final-integration review: rebuild used to silently drop them,
/// resetting the Beta feedback rerank prior to neutral and erasing mined
/// expansions despite the documented never-expire contract.
#[test]
fn rebuild_preserves_learning_state() {
    let home = tempdir().expect("tempdir");

    let save = run_json(
        &home,
        &[
            "save",
            "--kind",
            "decision",
            "postgres advisory locks for migration ordering",
        ],
    );
    let memory_id = save["id"].as_str().expect("save id").to_string();

    // Tracked search writes the retrieval_log row.
    let search = run_json(&home, &["search", "advisory lock"]);
    let query_id = search["query_id"].as_str().expect("query_id").to_string();

    // Feedback writes feedback_events provenance + the feedback counter.
    run_json(&home, &["feedback", &query_id, "--used", &memory_id]);

    // Seed one mined expansion directly (mine needs >= 2 supporting pairs).
    {
        let conn = open_db(&home);
        conn.execute(
            "INSERT INTO query_expansions(term, expansion, support, last_mined) \
             VALUES('pool', 'connection', 3, '2026-06-01T00:00:00Z')",
            [],
        )
        .expect("seed expansion");
    }

    run_rebuild(&home);

    let conn = open_db(&home);
    let used: i64 = conn
        .query_row(
            "SELECT used_count FROM feedback WHERE memory_id = ?1",
            [&memory_id],
            |r| r.get(0),
        )
        .expect("feedback counter must survive rebuild");
    assert_eq!(used, 1, "used_count spot value");

    assert_eq!(
        count(&conn, "SELECT count(*) FROM retrieval_log"),
        1,
        "retrieval_log rows must survive rebuild"
    );
    let logged_query: String = conn
        .query_row(
            "SELECT query FROM retrieval_log WHERE query_id = ?1",
            [&query_id],
            |r| r.get(0),
        )
        .expect("retrieval_log row must survive rebuild");
    assert_eq!(logged_query, "advisory lock", "retrieval_log spot value");

    assert_eq!(
        count(&conn, "SELECT count(*) FROM feedback_events"),
        1,
        "feedback_events rows must survive rebuild"
    );
    let verdict: String = conn
        .query_row(
            "SELECT verdict FROM feedback_events WHERE query_id = ?1 AND memory_id = ?2",
            rusqlite::params![query_id, memory_id],
            |r| r.get(0),
        )
        .expect("feedback_events row must survive rebuild");
    assert_eq!(verdict, "used", "feedback_events spot value");

    let support: i64 = conn
        .query_row(
            "SELECT support FROM query_expansions \
              WHERE term = 'pool' AND expansion = 'connection'",
            [],
            |r| r.get(0),
        )
        .expect("query_expansions row must survive rebuild");
    assert_eq!(support, 3, "query_expansions spot value");
}

/// Regression: a `comemory.db` written by a pre-v4 binary is attached raw
/// (`ATTACH DATABASE`, never migrated) during rebuild, so its 12-column
/// `code_symbols` table
/// used to break the `SELECT *` copy into the new 14-column schema
/// ("table main.code_symbols has 14 columns but 12 values were supplied"),
/// aborting the whole rebuild. The copy now lists columns explicitly and
/// synthesizes the v4 access columns (`access_count = 0`, `last_accessed =
/// indexed_at` — the same defaults migration 0004 backfills).
///
/// The same v3-shaped source DB also exercises the learning-table guards:
/// `feedback` (v2) and `retrieval_log` (v3, no `duration_ms` yet) must be
/// copied — with `duration_ms` defaulted to NULL — while the absent v5
/// tables (`feedback_events`, `query_expansions`) must be skipped without
/// aborting the rebuild.
#[test]
fn rebuild_succeeds_against_pre_v4_old_db() {
    let home = tempdir().expect("tempdir");
    std::fs::create_dir_all(home.path().join("memories")).expect("mkdir memories");

    // Register the process-global sqlite-vec extension (the 0002 DDL creates
    // vec0 vtabs) before replaying the old migrations on a raw connection.
    let scratch = tempdir().expect("scratch dir");
    drop(
        comemory::store::connection::open(scratch.path().join("scratch.db"))
            .expect("register sqlite-vec"),
    );

    let conn = Connection::open(home.path().join("comemory.db")).expect("open raw");
    conn.execute_batch(comemory::store::migrate::M_BOOTSTRAP)
        .expect("0001");
    conn.execute_batch(comemory::store::migrate::M_V2)
        .expect("0002");
    conn.execute_batch(comemory::store::migrate::M_V3)
        .expect("0003");
    conn.execute_batch(
        "INSERT INTO schema_meta(key, value) VALUES
            ('0002_v2_tables','1'), ('0003_stats_tables','1'), ('version','3');
         INSERT INTO code_symbols(id, repo, path, blob_oid, symbol, kind, lang,
                                  line_start, line_end, snippet, simhash, indexed_at)
         VALUES (1,'demo','src/lib.rs','beef0000','old_fn','function','rust',
                 1,2,'fn old_fn() {}',7,'2026-01-02T00:00:00Z');
         INSERT INTO code_fts(symbol_id, symbol, snippet, path_tokens)
         VALUES (1,'old_fn','fn old_fn() {}','src lib rs');
         INSERT INTO indexed_files(repo, path, blob_oid, indexed_at)
         VALUES ('demo','src/lib.rs','beef0000','2026-01-02T00:00:00Z');
         INSERT INTO feedback(memory_id, used_count, irrelevant_count, last_used)
         VALUES ('a1b2c3d4', 2, 1, '2026-01-03T00:00:00Z');
         INSERT INTO retrieval_log(query_id, query, returned_ids, at)
         VALUES ('q-20260103-aabbccdd','old query','a1b2c3d4','2026-01-03T00:00:00Z');",
    )
    .expect("seed v3 rows");
    drop(conn);

    run_rebuild(&home);

    let conn = open_db_with_vec(&home);
    let (access, last): (i64, String) = conn
        .query_row(
            "SELECT access_count, last_accessed FROM code_symbols WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("copied code_symbols row with synthesized v4 columns");
    assert_eq!(access, 0, "synthesized access_count must default to 0");
    assert_eq!(
        last, "2026-01-02T00:00:00Z",
        "synthesized last_accessed must fall back to indexed_at"
    );
    assert_eq!(count(&conn, "SELECT count(*) FROM code_fts"), 1);
    assert_eq!(count(&conn, "SELECT count(*) FROM indexed_files"), 1);

    // Learning tables present in the v3 source must be carried over;
    // retrieval_log.duration_ms (added in v5) must be defaulted to NULL.
    let (used, irrelevant): (i64, i64) = conn
        .query_row(
            "SELECT used_count, irrelevant_count FROM feedback WHERE memory_id = 'a1b2c3d4'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("v3 feedback counters must survive rebuild");
    assert_eq!((used, irrelevant), (2, 1));
    let duration: Option<i64> = conn
        .query_row(
            "SELECT duration_ms FROM retrieval_log WHERE query_id = 'q-20260103-aabbccdd'",
            [],
            |r| r.get(0),
        )
        .expect("v3 retrieval_log row must survive rebuild");
    assert_eq!(duration, None, "pre-v5 duration_ms must default to NULL");
    // The v5 tables are absent from the source: the guard must skip them
    // (leaving the freshly-migrated tables empty), not abort the rebuild.
    assert_eq!(count(&conn, "SELECT count(*) FROM feedback_events"), 0);
    assert_eq!(count(&conn, "SELECT count(*) FROM query_expansions"), 0);
}

/// When the rebuild cannot complete (e.g. the tmp DB path is blocked by a
/// pre-existing directory), the original `comemory.db` must be left intact
/// and the tmp artefact must be cleaned up.
///
/// We trigger the failure by pre-creating a *directory* at the tmp path
/// before running rebuild. SQLite cannot open a directory as a database, so
/// `connection::open(tmp_path)` returns an error immediately — well before
/// the rename that would clobber the live DB.
#[test]
fn rebuild_does_not_destroy_on_error() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "original body"]);

    // Record the original DB's memory count to compare after the failed rebuild.
    let original_count = {
        let conn = open_db(&home);
        count(&conn, "SELECT count(*) FROM memories")
    };

    // Block the tmp path with a directory so `connection::open` cannot
    // create the tmp DB — rebuild must return a non-zero exit code without
    // touching the live `comemory.db`.
    let tmp_path = home.path().join("comemory.db.rebuild.tmp");
    std::fs::create_dir_all(&tmp_path).expect("create dir at tmp path");

    // Rebuild must fail.
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["rebuild"])
        .assert()
        .failure();

    // The original DB must still be intact: it must open and have the same
    // memory count as before.
    let conn = open_db(&home);
    let after_count = count(&conn, "SELECT count(*) FROM memories");
    assert_eq!(
        after_count, original_count,
        "original DB must be intact after a failed rebuild"
    );
}
