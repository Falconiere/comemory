//! Tests for [`comemory::eval::runner`] — the golden-set eval driver and
//! its no-pollution invariant (measurement must not write tracking rows).

use comemory::eval::golden::GoldenPair;
use comemory::eval::runner::run_eval;

/// Build a db with three lexically distinct memories. Returns the tempdir
/// guard and the connection.
fn seeded() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    let rows: &[(&str, &str, i64)] = &[
        ("aaaa0001", "postgres pool exhausted advisory lock fix", 1),
        (
            "aaaa0002",
            "tokio runtime shutdown ordering bug",
            u32::MAX as i64,
        ),
        ("aaaa0003", "clap derive global flag placement note", -1),
    ];
    for (id, body, sim) in rows {
        conn.execute(
            "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                                  body, created_at, updated_at, md_path, simhash)
             VALUES (?1, ?1, 'note', 'd', 'f', 3, 1, ?1, ?2,
                     '2026-06-09T00:00:00Z', '2026-06-09T00:00:00Z', ?1, ?3)",
            rusqlite::params![id, body, sim],
        )
        .expect("insert memory");
        conn.execute(
            "INSERT INTO memory_fts(memory_id, body, tags) VALUES (?1, ?2, '')",
            rusqlite::params![id, body],
        )
        .expect("insert fts");
    }
    (dir, conn)
}

#[test]
fn run_eval_scores_obvious_lexical_match_perfectly() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let pairs = vec![GoldenPair {
        query: "postgres pool exhausted".into(),
        relevant: vec!["aaaa0001".into()],
    }];
    let report = run_eval(&cfg, &conn, &pairs, 3).expect("run_eval");
    assert_eq!(report.k, 3);
    assert_eq!(report.queries, 1);
    assert_eq!(report.recall_at_k, 1.0);
    assert_eq!(report.mrr, 1.0);
    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].rank_of_first_hit, Some(1));
    assert_eq!(report.results[0].returned[0], "aaaa0001");
}

#[test]
fn run_eval_misses_score_zero_and_sort_worst_first() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let pairs = vec![
        GoldenPair {
            query: "postgres pool exhausted".into(),
            relevant: vec!["aaaa0001".into()],
        },
        GoldenPair {
            query: "zebra quantum nonsense".into(),
            relevant: vec!["aaaa0002".into()],
        },
    ];
    let report = run_eval(&cfg, &conn, &pairs, 3).expect("run_eval");
    assert_eq!(report.queries, 2);
    assert_eq!(report.recall_at_k, 0.5);
    assert_eq!(report.mrr, 0.5);
    assert_eq!(
        report.results[0].query, "zebra quantum nonsense",
        "worst recall must sort first"
    );
    assert_eq!(report.results[0].rank_of_first_hit, None);
    assert_eq!(report.results[0].recall, 0.0);
}

#[test]
fn run_eval_does_not_pollute_tracking_state() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let snapshot = |conn: &rusqlite::Connection| -> (Vec<(String, i64)>, i64) {
        let mut stmt = conn
            .prepare("SELECT id, access_count FROM memories ORDER BY id")
            .expect("prepare");
        let counts: Vec<(String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");
        let logged: i64 = conn
            .query_row("SELECT count(*) FROM retrieval_log", [], |r| r.get(0))
            .expect("count");
        (counts, logged)
    };
    let before = snapshot(&conn);
    let pairs = vec![GoldenPair {
        query: "postgres pool exhausted".into(),
        relevant: vec!["aaaa0001".into()],
    }];
    run_eval(&cfg, &conn, &pairs, 3).expect("run_eval");
    let after = snapshot(&conn);
    assert_eq!(
        before, after,
        "eval must not bump access_count or write retrieval_log"
    );
}
