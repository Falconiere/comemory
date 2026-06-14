//! Task 14: `comemory rebuild` — part 2.
//!
//! Covers: learning-state preservation, v6 code-graph state preservation,
//! pre-v4 DB compatibility, and rollback-on-error safety.

#[path = "common/cli_rebuild_support.rs"]
mod support;
#[path = "common/vectors.rs"]
mod vectors;

use assert_cmd::Command;
use tempfile::tempdir;

use support::{count, open_db, open_db_with_vec, run_rebuild, run_save};

/// Run a `--json` subcommand against `home` and parse its stdout envelope.
fn run_json(home: &tempfile::TempDir, args: &[&str]) -> serde_json::Value {
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

/// Regression for the M3 final-integration review: every piece of v6
/// code-graph state must survive a rebuild. The old copy predated v6 and
/// silently dropped `rank_score` + `parent_id` (orphaning cAST chunks as
/// `name#n` rows forever), the mined `co_changed`/`imports` edges, the
/// `repo_marker.last_mined_commit` cursor, the whole `code_feedback`
/// table, `retrieval_log.repo/kind/source`, `feedback_events.target_kind`,
/// and the per-repo `code_format:<repo>` stamps.
#[test]
fn rebuild_preserves_v6_code_graph_state() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "v6 state preservation body"]);

    // Ingest a chunked symbol (parent + one cAST chunk child) through the
    // production JSONL path so parent_id is assigned the real way.
    let oid = "feed000000000000000000000000000000000000";
    let parent = serde_json::json!({
        "repo": "myrepo", "path": "src/lib.rs", "blob_oid": oid,
        "symbol": "preserved_fn", "kind": "function", "lang": "rust",
        "line_start": 1_u32, "line_end": 2_u32,
        "snippet": "fn preserved_fn() {}", "simhash": 0_i64,
        "embedding": vectors::vector("v6-parent", 768),
    });
    let chunk = serde_json::json!({
        "repo": "myrepo", "path": "src/lib.rs", "blob_oid": oid,
        "symbol": "preserved_fn#1", "kind": "function", "lang": "rust",
        "line_start": 3_u32, "line_end": 4_u32,
        "snippet": "chunk body", "simhash": 0_i64,
        "embedding": vectors::vector("v6-chunk", 768),
        "parent_symbol": "preserved_fn", "chunk_index": 1_u32,
    });
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["ingest-code"])
        .write_stdin(format!("{parent}\n{chunk}\n"))
        .assert()
        .success();

    // Seed the graph state index-code's materialize pass would produce:
    // a projected PageRank score, weighted co_changed + imports edges,
    // and the mining cursor.
    {
        let conn = open_db_with_vec(&home);
        conn.execute_batch(
            "UPDATE code_symbols SET rank_score = 0.7 WHERE repo = 'myrepo';
             INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, weight, created_at)
             VALUES ('file','file:myrepo:src/lib.rs','file','file:myrepo:src/other.rs',
                     'co_changed',3,'2026-06-10T00:00:00Z'),
                    ('file','file:myrepo:src/lib.rs','file','file:myrepo:src/other.rs',
                     'imports',1,'2026-06-10T00:00:00Z');
             INSERT INTO repo_marker(repo, last_head, last_indexed_at, last_mined_commit)
             VALUES ('myrepo','head0000',NULL,'cursor00');",
        )
        .expect("seed graph state");
    }

    // Tracked code search (writes the retrieval_log row with
    // source='search-code' + the repo/lang filters) followed by code
    // feedback (writes code_feedback + a target_kind='code' event).
    let search = run_json(
        &home,
        &[
            "search-code",
            "preserved",
            "--repo",
            "myrepo",
            "--lang",
            "rust",
        ],
    );
    let query_id = search["query_id"].as_str().expect("query_id").to_string();
    let parent_id: i64 = {
        let conn = open_db(&home);
        conn.query_row(
            "SELECT id FROM code_symbols WHERE symbol = 'preserved_fn'",
            [],
            |r| r.get(0),
        )
        .expect("parent id")
    };
    run_json(
        &home,
        &["feedback", &query_id, "--used-code", &parent_id.to_string()],
    );

    run_rebuild(&home);

    let conn = open_db_with_vec(&home);
    // Chunk → parent pointer and the projected rank survive.
    let (chunk_parent, rank): (i64, f64) = conn
        .query_row(
            "SELECT c.parent_id, c.rank_score FROM code_symbols c \
              WHERE c.symbol = 'preserved_fn#1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("chunk row");
    assert_eq!(chunk_parent, parent_id, "parent_id must survive rebuild");
    assert!(
        (rank - 0.7).abs() < 1e-12,
        "rank_score must survive rebuild"
    );
    // Both mined edge kinds survive with their weights.
    let (co_weight, import_count): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT weight FROM edges WHERE rel='co_changed'
                      AND src_id='file:myrepo:src/lib.rs'),
                    (SELECT count(*) FROM edges WHERE rel='imports'
                      AND src_id='file:myrepo:src/lib.rs')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("edges");
    assert_eq!(co_weight, 3, "co_changed weight must survive rebuild");
    assert_eq!(import_count, 1, "imports edge must survive rebuild");
    // Identity-keyed code feedback survives.
    let used: i64 = conn
        .query_row(
            "SELECT used_count FROM code_feedback \
              WHERE repo='myrepo' AND path='src/lib.rs' AND symbol='preserved_fn'",
            [],
            |r| r.get(0),
        )
        .expect("code_feedback row");
    assert_eq!(used, 1, "code_feedback must survive rebuild");
    // retrieval_log keeps its v6 filter/source columns.
    let (source, repo, kind): (String, String, String) = conn
        .query_row(
            "SELECT source, repo, kind FROM retrieval_log WHERE query_id = ?1",
            [&query_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("retrieval_log row");
    assert_eq!(
        (source.as_str(), repo.as_str(), kind.as_str()),
        ("search-code", "myrepo", "rust"),
        "retrieval_log source/repo/kind must survive rebuild"
    );
    // feedback_events keeps target_kind so harvest/mine guards still hold.
    let target_kind: String = conn
        .query_row(
            "SELECT target_kind FROM feedback_events WHERE query_id = ?1",
            [&query_id],
            |r| r.get(0),
        )
        .expect("feedback_events row");
    assert_eq!(target_kind, "code", "target_kind must survive rebuild");
    // Mining cursor + per-repo format stamp survive.
    let cursor: String = conn
        .query_row(
            "SELECT last_mined_commit FROM repo_marker WHERE repo='myrepo'",
            [],
            |r| r.get(0),
        )
        .expect("repo_marker row");
    assert_eq!(cursor, "cursor00", "last_mined_commit must survive rebuild");
    let stamp: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'code_format:myrepo'",
            [],
            |r| r.get(0),
        )
        .expect("per-repo format stamp");
    assert_eq!(
        stamp, "2",
        "code_format stamp must survive rebuild or the next index-code wipes code_vec"
    );
}

/// Step 4 (v8 auto-reinforcement): the reinforcement state earned by the
/// co-activation reward is DB-only — markdown has no source for it — so
/// `comemory rebuild` must carry it forward exactly like the v6 mined edges
/// and the feedback counters. This seeds the three reinforcement channels
/// the way `graph::coactivate::reward_pair` + `feedback::record_implicit_used`
/// write them — a weighted `co_activated` memory→file edge, an
/// `auto_coactivation`-provenance `feedback_events` row under the
/// `auto-coactivation` sentinel query id, and the `feedback.used_count` bump
/// that minted it — then asserts all three survive a rebuild. Without Step 4
/// the rebuild would silently erase the accumulated reinforcement.
#[test]
fn rebuild_preserves_v8_reinforcement_state() {
    let home = tempdir().expect("tempdir");

    let save = run_json(
        &home,
        &[
            "save",
            "--kind",
            "decision",
            "--repo",
            "myrepo",
            "co-activation reward reinforces this memory",
        ],
    );
    let memory_id = save["id"].as_str().expect("save id").to_string();

    // Seed the earned reinforcement state the materialize/coactivate path
    // produces: a weighted co_activated memory→file edge (canonical
    // `file:<repo>:<path>` dst_id), the implicit feedback_events row tagged
    // provenance='auto_coactivation' under the sentinel query id, and the
    // feedback.used_count bump that minted it.
    let file_node = "file:myrepo:src/lib.rs";
    {
        let conn = open_db(&home);
        conn.execute(
            "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, weight, created_at) \
             VALUES ('memory', ?1, 'file', ?2, 'co_activated', 5, '2026-06-12T00:00:00Z')",
            rusqlite::params![memory_id, file_node],
        )
        .expect("seed co_activated edge");
        conn.execute(
            "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind, provenance) \
             VALUES ('auto-coactivation', ?1, 'used', '2026-06-12T00:00:00Z', 'memory', \
                     'auto_coactivation')",
            rusqlite::params![memory_id],
        )
        .expect("seed implicit feedback_events row");
        conn.execute(
            "INSERT INTO feedback(memory_id, used_count, irrelevant_count, last_used) \
             VALUES (?1, 3, 0, '2026-06-12T00:00:00Z')",
            rusqlite::params![memory_id],
        )
        .expect("seed feedback counter");
    }

    run_rebuild(&home);

    let conn = open_db(&home);
    // The co_activated edge survives with its accumulated weight.
    let edge_weight: i64 = conn
        .query_row(
            "SELECT weight FROM edges WHERE rel = 'co_activated' \
               AND src_kind = 'memory' AND src_id = ?1 \
               AND dst_kind = 'file' AND dst_id = ?2",
            rusqlite::params![memory_id, file_node],
            |r| r.get(0),
        )
        .expect("co_activated edge must survive rebuild");
    assert_eq!(edge_weight, 5, "co_activated weight must survive rebuild");

    // The implicit feedback_events row survives WITH its provenance tag.
    let (verdict, provenance): (String, String) = conn
        .query_row(
            "SELECT verdict, provenance FROM feedback_events \
               WHERE query_id = 'auto-coactivation' AND memory_id = ?1",
            rusqlite::params![memory_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("implicit feedback_events row must survive rebuild");
    assert_eq!(verdict, "used", "implicit verdict must survive rebuild");
    assert_eq!(
        provenance, "auto_coactivation",
        "auto_coactivation provenance must survive rebuild"
    );

    // The feedback counter (the implicit reward lands here) survives.
    let used: i64 = conn
        .query_row(
            "SELECT used_count FROM feedback WHERE memory_id = ?1",
            rusqlite::params![memory_id],
            |r| r.get(0),
        )
        .expect("feedback counter must survive rebuild");
    assert_eq!(used, 3, "implicit used_count must survive rebuild");
}

/// Step 4: a pre-0008 source DB whose `feedback_events` table lacks the v8
/// `provenance` column must still rebuild — the copy probes for the column
/// and defaults a missing one to `'manual'` (the same backfill the 0008
/// migration applies). Replaying the migration chain on a raw connection up
/// to v7 is not viable (0004+ needs the custom FTS5 identifier tokenizer,
/// which is only registered by `connection::open`), so the v8 source is
/// opened fully migrated and then its `provenance` column is dropped to
/// reproduce the pre-v8 `feedback_events` shape that the rebuild copy's
/// structural `old_column_exists` probe keys off — the probe reads
/// `pragma_table_info`, never `schema_meta`, so the dropped column is an
/// exact stand-in for a genuinely pre-v8 source.
#[test]
fn rebuild_defaults_missing_provenance_to_manual() {
    let home = tempdir().expect("tempdir");

    // Save a memory so the DB exists and is fully migrated to v8.
    let save = run_json(
        &home,
        &["save", "--kind", "note", "pre-v8 provenance probe"],
    );
    let memory_id = save["id"].as_str().expect("save id").to_string();

    // Seed a feedback_events row, then DROP the v8 provenance column so the
    // source looks pre-0008 to the rebuild copy's structural column probe.
    {
        let conn = open_db(&home);
        conn.execute(
            "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind, provenance) \
             VALUES ('q-20260103-aabbccdd', ?1, 'used', '2026-01-03T00:00:00Z', 'memory', \
                     'auto_coactivation')",
            rusqlite::params![memory_id],
        )
        .expect("seed feedback_events row");
        conn.execute_batch("ALTER TABLE feedback_events DROP COLUMN provenance;")
            .expect("drop provenance column to simulate pre-v8 source");
        let has_prov: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('feedback_events') \
                   WHERE name = 'provenance'",
                [],
                |r| r.get(0),
            )
            .expect("probe provenance");
        assert_eq!(has_prov, 0, "source must now lack the v8 provenance column");
    }

    run_rebuild(&home);

    let conn = open_db_with_vec(&home);
    let provenance: String = conn
        .query_row(
            "SELECT provenance FROM feedback_events WHERE memory_id = ?1",
            rusqlite::params![memory_id],
            |r| r.get(0),
        )
        .expect("pre-v8 feedback_events row must survive rebuild");
    assert_eq!(
        provenance, "manual",
        "missing pre-v8 provenance must default to 'manual'"
    );
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

    let conn = rusqlite::Connection::open(home.path().join("comemory.db")).expect("open raw");
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
