//! Tests for [`comemory::retrieval::pipeline::search`] — the end-to-end
//! route → rerank → diversify → top-k path plus best-effort access
//! tracking and query logging.

use comemory::retrieval::pipeline::{PageWindow, SearchOptions, search};
use comemory::simhash::{NEAR_DUP_HAMMING, hamming64};

fn seeded() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, simhash)
         VALUES ('aaaa0001','a','note','d','f',3,1,'h1','sqlite busy timeout fix for pool',
                 '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/1.md',1);
         INSERT INTO memory_fts(memory_id, body, tags)
         VALUES ('aaaa0001','sqlite busy timeout fix for pool','');",
    )
    .expect("seed");
    (dir, conn)
}

#[test]
fn search_returns_reranked_diversified_hits() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let run = search(
        &cfg,
        &conn,
        "sqlite busy",
        None,
        None,
        None,
        SearchOptions {
            track: false,
            source: "search",
            window: PageWindow::top_k(&cfg),
        },
    )
    .expect("search");
    assert_eq!(run.hits.len(), 1);
    assert_eq!(run.hits[0].memory_id, "aaaa0001");
    assert!(run.hits[0].parts.final_score > 0.0);
}

#[test]
fn retrieval_hit_bumps_access_tracking() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    search(
        &cfg,
        &conn,
        "sqlite busy",
        None,
        None,
        None,
        SearchOptions {
            track: true,
            source: "search",
            window: PageWindow::top_k(&cfg),
        },
    )
    .expect("search");
    let (count, last): (i64, String) = conn
        .query_row(
            "SELECT access_count, last_accessed FROM memories WHERE id='aaaa0001'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!(count, 1);
    assert!(
        last.as_str() > "2026-06-09T00:00:00Z",
        "last_accessed updated, got {last}"
    );
}

#[test]
fn access_tracking_failure_does_not_break_reads() {
    let (_d, conn) = seeded();
    // Make every write fail: query_only rejects the access-tracking UPDATE
    // and the retrieval_log INSERT while leaving the read path untouched.
    conn.pragma_update(None, "query_only", true)
        .expect("pragma");
    let cfg = comemory::config::Config::defaults();
    let run = search(
        &cfg,
        &conn,
        "sqlite busy",
        None,
        None,
        None,
        SearchOptions {
            track: true,
            source: "search",
            window: PageWindow::top_k(&cfg),
        },
    )
    .expect("search must succeed when access tracking cannot write");
    assert_eq!(run.hits.len(), 1);
    assert_eq!(run.hits[0].memory_id, "aaaa0001");
    // Query logging is best-effort too: a failed INSERT yields no id.
    assert!(run.query_id.is_none(), "logging failed, id must be None");
    // The bump itself was skipped, not silently rerouted somewhere else.
    let count: i64 = conn
        .query_row(
            "SELECT access_count FROM memories WHERE id='aaaa0001'",
            [],
            |r| r.get(0),
        )
        .expect("row");
    assert_eq!(count, 0);
}

#[test]
fn search_with_track_logs_one_retrieval_log_row() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let run = search(
        &cfg,
        &conn,
        "sqlite busy",
        None,
        None,
        None,
        SearchOptions {
            track: true,
            source: "search",
            window: PageWindow::top_k(&cfg),
        },
    )
    .expect("search");
    let qid = run.query_id.expect("query_id present when tracking");
    assert!(comemory::stats::feedback::is_valid_query_id(&qid));
    let (q, ids, dur): (String, String, Option<i64>) = conn
        .query_row(
            "SELECT query, returned_ids, duration_ms FROM retrieval_log WHERE query_id = ?1",
            [&qid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("logged");
    assert_eq!(q, "sqlite busy");
    let parsed: Vec<String> = serde_json::from_str(&ids).expect("json ids");
    assert_eq!(parsed.len(), run.hits.len());
    assert_eq!(parsed[0], "aaaa0001");
    assert!(dur.is_some());
}

#[test]
fn search_with_filters_logs_repo_kind_and_source() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let run = search(
        &cfg,
        &conn,
        "sqlite busy",
        None,
        Some("d"),
        Some("note"),
        SearchOptions {
            track: true,
            source: "search",
            window: PageWindow::top_k(&cfg),
        },
    )
    .expect("search");
    let qid = run.query_id.expect("query_id present when tracking");
    let (repo, kind, source): (Option<String>, Option<String>, String) = conn
        .query_row(
            "SELECT repo, kind, source FROM retrieval_log WHERE query_id = ?1",
            [&qid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("logged");
    assert_eq!(repo.as_deref(), Some("d"), "repo filter logged verbatim");
    assert_eq!(kind.as_deref(), Some("note"), "kind filter logged verbatim");
    assert_eq!(source, "search");
}

#[test]
fn search_without_filters_logs_null_repo_and_kind() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let run = search(
        &cfg,
        &conn,
        "sqlite busy",
        None,
        None,
        None,
        SearchOptions {
            track: true,
            source: "context",
            window: PageWindow::top_k(&cfg),
        },
    )
    .expect("search");
    let qid = run.query_id.expect("query_id present when tracking");
    let (repo, kind, source): (Option<String>, Option<String>, String) = conn
        .query_row(
            "SELECT repo, kind, source FROM retrieval_log WHERE query_id = ?1",
            [&qid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("logged");
    assert_eq!(repo, None, "no repo filter must log NULL");
    assert_eq!(kind, None, "no kind filter must log NULL");
    assert_eq!(source, "context", "caller-declared source logged verbatim");
}

#[test]
fn search_without_track_logs_nothing_and_freezes_access() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    let before: i64 = conn
        .query_row(
            "SELECT access_count FROM memories WHERE id='aaaa0001'",
            [],
            |r| r.get(0),
        )
        .expect("row");
    for _ in 0..2 {
        let run = search(
            &cfg,
            &conn,
            "sqlite busy",
            None,
            None,
            None,
            SearchOptions {
                track: false,
                source: "search",
                window: PageWindow::top_k(&cfg),
            },
        )
        .expect("search");
        assert!(run.query_id.is_none(), "no query_id when track is off");
        assert_eq!(run.hits.len(), 1);
    }
    let logged: i64 = conn
        .query_row("SELECT count(*) FROM retrieval_log", [], |r| r.get(0))
        .expect("count");
    assert_eq!(logged, 0, "track:false must not write retrieval_log");
    let after: i64 = conn
        .query_row(
            "SELECT access_count FROM memories WHERE id='aaaa0001'",
            [],
            |r| r.get(0),
        )
        .expect("row");
    assert_eq!(after, before, "track:false must not bump access_count");
}

#[test]
fn pipeline_cuts_to_configured_top_k() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    // 15 distinct memories matching the single term "sqlite". SimHashes are
    // spread via a golden-ratio multiply so no pair collapses as a near-dup
    // (the loop asserts pairwise Hamming > NEAR_DUP_HAMMING to keep the
    // fixture honest).
    let mut sims: Vec<u64> = Vec::new();
    for i in 0..15u64 {
        let sim = (i + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for prev in &sims {
            assert!(
                hamming64(*prev, sim) > NEAR_DUP_HAMMING,
                "fixture simhashes must not collapse as near-dups"
            );
        }
        sims.push(sim);
        let id = format!("bbbb{i:04}");
        let body = format!("sqlite topic number {i}");
        conn.execute(
            "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                                  body, created_at, updated_at, md_path, simhash)
             VALUES (?1, ?2, 'note', 'd', 'f', 3, 1, ?3, ?4,
                     '2026-06-09T00:00:00Z', '2026-06-09T00:00:00Z', ?5, ?6)",
            rusqlite::params![
                id,
                format!("s{i}"),
                format!("h{i}"),
                body,
                format!("m/{i}.md"),
                sim as i64
            ],
        )
        .expect("seed memory");
        conn.execute(
            "INSERT INTO memory_fts(memory_id, body, tags) VALUES (?1, ?2, '')",
            rusqlite::params![id, body],
        )
        .expect("seed fts");
    }
    let cfg = comemory::config::Config::defaults();
    assert_eq!(
        cfg.retrieval.top_k, 12,
        "default top_k expected by this test"
    );
    let run = search(
        &cfg,
        &conn,
        "sqlite",
        None,
        None,
        None,
        SearchOptions {
            track: false,
            source: "search",
            window: PageWindow::top_k(&cfg),
        },
    )
    .expect("search");
    assert_eq!(run.hits.len(), 12, "pipeline must cut to top_k");
}
