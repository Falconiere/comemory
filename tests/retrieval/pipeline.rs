//! Tests for [`comemory::retrieval::pipeline::search`] — the end-to-end
//! route → rerank → diversify → top-k path plus best-effort access
//! tracking.

use comemory::retrieval::pipeline::search;
use comemory::simhash::{hamming64, NEAR_DUP_HAMMING};

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
    let out = search(&cfg, &conn, "sqlite busy", None, None, None).expect("search");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].memory_id, "aaaa0001");
    assert!(out[0].parts.final_score > 0.0);
}

#[test]
fn retrieval_hit_bumps_access_tracking() {
    let (_d, conn) = seeded();
    let cfg = comemory::config::Config::defaults();
    search(&cfg, &conn, "sqlite busy", None, None, None).expect("search");
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
    // while leaving the read path untouched.
    conn.pragma_update(None, "query_only", true)
        .expect("pragma");
    let cfg = comemory::config::Config::defaults();
    let out = search(&cfg, &conn, "sqlite busy", None, None, None)
        .expect("search must succeed when access tracking cannot write");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].memory_id, "aaaa0001");
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
    let out = search(&cfg, &conn, "sqlite", None, None, None).expect("search");
    assert_eq!(out.len(), 12, "pipeline must cut to top_k");
}
