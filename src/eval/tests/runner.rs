#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::eval::runner`] — the golden-set eval driver and
//! its no-pollution invariant (measurement must not write tracking rows).

use comemory::eval::golden::GoldenPair;
use comemory::eval::runner::run_eval;

/// Insert one searchable memory (`memories` row + FTS row) with an
/// explicit kind and the *real* simhash of `body` — the same
/// `simhash::of_body(..) as i64` the production save path stores, so the
/// near-dup/diversify leg sees production-shaped values instead of
/// hand-picked sentinels.
fn insert_memory(conn: &rusqlite::Connection, id: &str, kind: &str, body: &str) {
    let sim = comemory::simhash::of_body(body) as i64;
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, simhash)
         VALUES (?1, ?1, ?2, 'd', 'f', 3, 1, ?1, ?3,
                 '2026-06-09T00:00:00Z', '2026-06-09T00:00:00Z', ?1, ?4)",
        rusqlite::params![id, kind, body, sim],
    )
    .expect("insert memory");
    conn.execute(
        "INSERT INTO memory_fts(memory_id, body, tags) VALUES (?1, ?2, '')",
        rusqlite::params![id, body],
    )
    .expect("insert fts");
}

/// Fixture guard: two bodies the assertions expect to co-exist in one
/// result list must sit outside the near-dup radius. Real simhashes replaced
/// the old hand-picked sentinels, so a body edit that pulls two rows inside
/// that radius has to fail here rather than as a mystery collapse downstream.
fn assert_not_near_dup(a: &str, b: &str) {
    let radius = comemory::config::Config::defaults().rank.near_dup_hamming;
    let d =
        comemory::simhash::hamming64(comemory::simhash::of_body(a), comemory::simhash::of_body(b));
    assert!(
        d > radius,
        "fixture bodies collapse as near-dups (hamming {d} <= {radius}): {a:?} / {b:?}"
    );
}

/// Build a db with three lexically distinct memories. Returns the tempdir
/// guard and the connection.
fn seeded() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    let rows: &[(&str, &str)] = &[
        ("aaaa0001", "postgres pool exhausted advisory lock fix"),
        ("aaaa0002", "tokio runtime shutdown ordering bug"),
        ("aaaa0003", "clap derive global flag placement note"),
    ];
    for (id, body) in rows {
        insert_memory(&conn, id, "note", body);
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
        repo: None,
        kind: None,
    }];
    let report = run_eval(&cfg, &conn, &pairs, 3).expect("run_eval");
    assert_eq!(report.k, 3);
    assert_eq!(report.queries, 1);
    assert_eq!(report.recall_at_k, 1.0);
    assert_eq!(report.mrr, 1.0);
    assert_eq!(
        (report.recall_ci, report.mrr_ci),
        ((1.0, 1.0), (1.0, 1.0)),
        "a single query carries no spread to bracket"
    );
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
            repo: None,
            kind: None,
        },
        GoldenPair {
            query: "zebra quantum nonsense".into(),
            relevant: vec!["aaaa0002".into()],
            repo: None,
            kind: None,
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

/// Insert one directed edge in the production `(kind, id)` addressing.
fn insert_edge(conn: &rusqlite::Connection, src: &str, dst: &str, rel: &str) {
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES('memory',?1,'memory',?2,?3,'t')",
        rusqlite::params![src, dst, rel],
    )
    .expect("insert edge");
}

/// A citer the lexical leg finds, a `derived_from` target that is dark for
/// the same query, and one unrelated memory. Returns the tempdir guard and
/// the connection.
fn edge_linked() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    insert_memory(
        &conn,
        "cccc0001",
        "note",
        "duplicate invoice notifications reached customers twice",
    );
    insert_memory(
        &conn,
        "cccc0002",
        "note",
        "reconciliation sweep double counted when two shifts overlapped",
    );
    insert_memory(&conn, "cccc0003", "note", "clap derive flag placement");
    insert_edge(&conn, "cccc0001", "cccc0002", "derived_from");
    (dir, conn)
}

/// `Config::defaults()` with the graph walk depth pinned to `hops`.
fn cfg_hops(hops: u32) -> comemory::config::Config {
    let mut cfg = comemory::config::Config::defaults();
    cfg.retrieval.graph_hops = hops;
    cfg
}

#[test]
fn graph_expansion_leg_is_visible_to_eval() {
    // AC-4: the golden target is lexically dark for its query and reachable
    // only by walking `derived_from` out of the memory the lexical leg does
    // find. Scoring the same pair with the walk disabled must therefore be
    // strictly worse — which is what lets `comemory tune` see the leg at all.
    let (_d, conn) = edge_linked();
    let pairs = vec![GoldenPair {
        query: "duplicate invoice notifications".into(),
        relevant: vec!["cccc0002".into()],
        repo: None,
        kind: None,
    }];
    let on = run_eval(&cfg_hops(2), &conn, &pairs, 3).expect("run_eval hops=2");
    let off = run_eval(&cfg_hops(0), &conn, &pairs, 3).expect("run_eval hops=0");

    assert!(
        off.recall_at_k < on.recall_at_k,
        "hops=0 must lose recall: {} !< {}",
        off.recall_at_k,
        on.recall_at_k
    );
    assert!(
        off.mrr < on.mrr,
        "hops=0 must lose mrr: {} !< {}",
        off.mrr,
        on.mrr
    );
    assert_eq!(
        off.results[0].rank_of_first_hit, None,
        "the target is lexically dark, so the walk is its only route"
    );
    assert!(
        on.results[0].returned.contains(&"cccc0002".to_string()),
        "the walk must surface the dark target: {:?}",
        on.results[0].returned
    );
}

#[test]
fn run_eval_brackets_its_point_estimates_reproducibly() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let pairs = vec![
        GoldenPair {
            query: "postgres pool exhausted".into(),
            relevant: vec!["aaaa0001".into()],
            repo: None,
            kind: None,
        },
        GoldenPair {
            query: "tokio runtime shutdown ordering".into(),
            relevant: vec!["aaaa0002".into()],
            repo: None,
            kind: None,
        },
        GoldenPair {
            query: "zebra quantum nonsense".into(),
            relevant: vec!["aaaa0003".into()],
            repo: None,
            kind: None,
        },
    ];
    let first = run_eval(&cfg, &conn, &pairs, 3).expect("run_eval");
    let second = run_eval(&cfg, &conn, &pairs, 3).expect("run_eval again");

    assert_eq!(
        (first.recall_ci, first.mrr_ci),
        (second.recall_ci, second.mrr_ci),
        "the bootstrap seed derives from the golden set, so reruns match"
    );
    for (label, point, (lo, hi)) in [
        ("recall", first.recall_at_k, first.recall_ci),
        ("mrr", first.mrr, first.mrr_ci),
    ] {
        assert!(
            lo <= point && point <= hi,
            "{label} interval [{lo}, {hi}] must contain the point estimate {point}"
        );
        assert!(
            lo < hi,
            "a mixed hit/miss set must produce a non-zero {label} width"
        );
    }
}

#[test]
fn run_eval_confidence_intervals_track_the_recall_cut() {
    // k is mixed into the bootstrap seed, so the same corpus scored at a
    // different cut does not reuse the same resample.
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let pairs = vec![
        GoldenPair {
            query: "postgres pool exhausted".into(),
            relevant: vec!["aaaa0001".into(), "aaaa0002".into()],
            repo: None,
            kind: None,
        },
        GoldenPair {
            query: "clap derive global flag".into(),
            relevant: vec!["aaaa0003".into()],
            repo: None,
            kind: None,
        },
    ];
    let at_one = run_eval(&cfg, &conn, &pairs, 1).expect("k=1");
    let at_three = run_eval(&cfg, &conn, &pairs, 3).expect("k=3");
    assert!(
        at_one.recall_at_k <= at_three.recall_at_k,
        "recall cannot fall as k grows: {} > {}",
        at_one.recall_at_k,
        at_three.recall_at_k
    );
    assert_eq!(
        at_three.recall_ci,
        run_eval(&cfg, &conn, &pairs, 3)
            .expect("k=3 again")
            .recall_ci,
        "same k must replay the same interval"
    );
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
        repo: None,
        kind: None,
    }];
    run_eval(&cfg, &conn, &pairs, 3).expect("run_eval");
    let after = snapshot(&conn);
    assert_eq!(
        before, after,
        "eval must not bump access_count or write retrieval_log"
    );
}

#[test]
fn run_eval_replays_kind_filter_from_the_pair() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    let decision_body = "postgres pool exhausted advisory lock fix";
    let note_body = "postgres pool exhausted incident note";
    insert_memory(&conn, "bbbb0001", "decision", decision_body);
    insert_memory(&conn, "bbbb0002", "note", note_body);
    // Both rows must survive the unfiltered assertion below, so they have to
    // stay outside the collapse radius on their real simhashes.
    assert_not_near_dup(decision_body, note_body);
    let cfg = comemory::config::Config::defaults();

    let filtered = vec![GoldenPair {
        query: "postgres pool exhausted".into(),
        relevant: vec!["bbbb0001".into()],
        repo: None,
        kind: Some("decision".into()),
    }];
    let report = run_eval(&cfg, &conn, &filtered, 3).expect("run_eval filtered");
    assert_eq!(report.recall_at_k, 1.0, "decision id must be reachable");
    assert!(
        !report.results[0].returned.contains(&"bbbb0002".to_string()),
        "kind filter must exclude the note hit: {:?}",
        report.results[0].returned
    );

    let unfiltered = vec![GoldenPair {
        query: "postgres pool exhausted".into(),
        relevant: vec!["bbbb0001".into()],
        repo: None,
        kind: None,
    }];
    let report = run_eval(&cfg, &conn, &unfiltered, 3).expect("run_eval unfiltered");
    let returned = &report.results[0].returned;
    assert!(
        returned.contains(&"bbbb0001".to_string()) && returned.contains(&"bbbb0002".to_string()),
        "without a kind filter both kinds must return: {returned:?}"
    );
}
