//! Tests for [`comemory::retrieval::rerank::rerank`].
//!
//! Seeds real `memories` / `feedback` / `edges` rows (all columns from
//! migration v4 present) and asserts that the bounded multiplicative
//! priors reorder, annotate, and explain candidates deterministically.

use comemory::retrieval::rerank::{Reranked, rerank};
use comemory::retrieval::router::{RoutedHit, Source};

fn open_seeded() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("comemory.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, access_count, last_accessed, simhash)
         VALUES
         ('aaaa0001','one','note','demo','f',3,1,'h1','first body',
          '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/1.md',0,'2026-06-09T00:00:00Z',1),
         ('aaaa0002','two','note','demo','f',5,1,'h2','second body',
          '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/2.md',0,'2026-06-09T00:00:00Z',2),
         ('aaaa0003','old','note','demo','f',3,1,'h3','third body',
          '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/3.md',0,'2026-06-09T00:00:00Z',3);
         INSERT INTO feedback(memory_id, used_count, irrelevant_count)
         VALUES ('aaaa0003', 0, 20);
         -- aaaa0002 supersedes aaaa0001
         INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0002','memory','aaaa0001','supersedes','2026-06-09T00:00:00Z');",
    )
    .expect("seed");
    (dir, conn)
}

fn hit(id: &str, score: f32) -> RoutedHit {
    RoutedHit {
        memory_id: id.into(),
        score,
        source: Source::Lexical,
        tier: 1,
    }
}

#[test]
fn priors_reorder_equal_relevance() {
    let (_d, conn) = open_seeded();
    let cfg = comemory::config::Config::defaults();
    let hits = vec![
        hit("aaaa0001", 1.0),
        hit("aaaa0002", 1.0),
        hit("aaaa0003", 1.0),
    ];
    let out: Vec<Reranked> = rerank(&conn, &cfg, &hits, None).expect("rerank");
    // quality-5 un-superseded memory first; superseded ×0.2 sinks below
    // the feedback-floored (0.5) downvoted one → aaaa0001 last.
    assert_eq!(out[0].memory_id, "aaaa0002");
    assert_eq!(out.last().expect("nonempty").memory_id, "aaaa0001");
}

#[test]
fn superseded_hit_is_annotated_and_penalized() {
    let (_d, conn) = open_seeded();
    let cfg = comemory::config::Config::defaults();
    let out = rerank(&conn, &cfg, &[hit("aaaa0001", 1.0)], None).expect("rerank");
    assert_eq!(out[0].superseded_by.as_deref(), Some("aaaa0002"));
    assert!((out[0].parts.supersede - 0.2).abs() < 1e-9);
    assert!(out[0].parts.final_score < 0.3);
}

#[test]
fn score_parts_multiply_to_final() {
    let (_d, conn) = open_seeded();
    let cfg = comemory::config::Config::defaults();
    let out = rerank(&conn, &cfg, &[hit("aaaa0002", 0.8)], None).expect("rerank");
    let p = &out[0].parts;
    let expect = f64::from(p.rrf) * p.activation * p.feedback * p.quality * p.supersede;
    assert!((p.final_score - expect).abs() < 1e-9);
}

#[test]
fn missing_or_soft_deleted_hits_are_dropped() {
    let (_d, conn) = open_seeded();
    conn.execute(
        "UPDATE memories SET deleted_at = '2026-06-09T01:00:00Z' WHERE id = 'aaaa0003'",
        [],
    )
    .expect("soft delete");
    let cfg = comemory::config::Config::defaults();
    let hits = vec![
        hit("aaaa0002", 1.0),
        hit("ffff9999", 1.0), // never existed
        hit("aaaa0003", 1.0), // soft-deleted
    ];
    let out = rerank(&conn, &cfg, &hits, None).expect("rerank");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].memory_id, "aaaa0002");
}

#[test]
fn soft_deleted_superseder_does_not_penalize() {
    let (_d, conn) = open_seeded();
    // Kill the superseder: its edge must stop punishing aaaa0001.
    conn.execute(
        "UPDATE memories SET deleted_at = '2026-06-09T01:00:00Z' WHERE id = 'aaaa0002'",
        [],
    )
    .expect("soft delete superseder");
    let cfg = comemory::config::Config::defaults();
    let out = rerank(&conn, &cfg, &[hit("aaaa0001", 1.0)], None).expect("rerank");
    assert_eq!(out[0].superseded_by, None);
    assert!((out[0].parts.supersede - 1.0).abs() < 1e-12);
}

#[test]
fn self_supersede_edge_does_not_penalize() {
    // Defense-in-depth: a hand-seeded self-edge (the writers refuse to
    // create one) must not annotate the memory as "superseded by itself"
    // or apply the 0.2 penalty.
    let (_d, conn) = open_seeded();
    conn.execute_batch(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0003','memory','aaaa0003','supersedes','2026-06-09T00:00:00Z');",
    )
    .expect("seed self edge");
    let cfg = comemory::config::Config::defaults();
    let out = rerank(&conn, &cfg, &[hit("aaaa0003", 1.0)], None).expect("rerank");
    assert_eq!(out[0].superseded_by, None, "self-edge must not annotate");
    assert!((out[0].parts.supersede - 1.0).abs() < 1e-12);
}

#[test]
fn malformed_last_accessed_scores_as_fresh() {
    let (_d, conn) = open_seeded();
    conn.execute(
        "UPDATE memories SET last_accessed = 'not-a-timestamp' WHERE id = 'aaaa0002'",
        [],
    )
    .expect("corrupt timestamp");
    let cfg = comemory::config::Config::defaults();
    // No error, and the garbage timestamp scores as fresh: access_count 0
    // + 0 days → activation 0 → boost exactly 1.0 (neutral).
    let out = rerank(&conn, &cfg, &[hit("aaaa0002", 1.0)], None).expect("rerank");
    assert!((out[0].parts.activation - 1.0).abs() < 1e-12);
}

#[test]
fn no_feedback_row_is_neutral() {
    let (_d, conn) = open_seeded();
    let cfg = comemory::config::Config::defaults();
    // aaaa0002 has no feedback row → Beta(1,3) neutral → boost exactly 1.0.
    let out = rerank(&conn, &cfg, &[hit("aaaa0002", 1.0)], None).expect("rerank");
    assert!((out[0].parts.feedback - 1.0).abs() < 1e-12);
}

#[test]
fn ties_break_on_memory_id_ascending() {
    let (_d, conn) = open_seeded();
    let cfg = comemory::config::Config::defaults();
    // Seed two rows with identical priors (same quality, no feedback,
    // not superseded) so their final scores tie exactly.
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, access_count, last_accessed, simhash)
         VALUES
         ('bbbb0001','tie-a','note','demo','f',3,1,'h4','tie body a',
          '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/4.md',0,'2026-06-09T00:00:00Z',4),
         ('bbbb0002','tie-b','note','demo','f',3,1,'h5','tie body b',
          '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/5.md',0,'2026-06-09T00:00:00Z',5);",
    )
    .expect("seed ties");
    // Present in descending-id order; equal scores must come back ascending.
    let hits = vec![hit("bbbb0002", 1.0), hit("bbbb0001", 1.0)];
    let out = rerank(&conn, &cfg, &hits, None).expect("rerank");
    assert_eq!(out[0].memory_id, "bbbb0001");
    assert_eq!(out[1].memory_id, "bbbb0002");
}

#[test]
fn priors_boost_on_normalized_scale_no_inversion() {
    // Raw candidate scores arrive on arbitrary per-branch scales; priors
    // multiplied onto them raw could invert boost direction. After
    // max-ratio normalization the priors multiply a positive [0,1]
    // relevance with within-pool ratios preserved, so boosts cannot
    // invert direction and no candidate is prior-immune at zero.
    let (_d, conn) = open_seeded();
    let cfg = comemory::config::Config::defaults();
    let hits = vec![
        hit("aaaa0001", 2.0), // worse lexical hit
        hit("aaaa0002", 8.0), // better lexical hit
    ];
    let out = rerank(&conn, &cfg, &hits, None).expect("rerank");
    let worse = out
        .iter()
        .find(|r| r.memory_id == "aaaa0001")
        .expect("worse");
    let better = out
        .iter()
        .find(|r| r.memory_id == "aaaa0002")
        .expect("better");
    assert_eq!(better.parts.rrf, 1.0);
    assert_eq!(worse.parts.rrf, 0.25);
    assert!(better.parts.final_score > worse.parts.final_score);
    // ratio preservation: the pool minimum keeps a nonzero score, so
    // priors can still reorder it (no prior-immune zero).
    assert!(worse.parts.final_score > 0.0);
    // invariant holds on the normalized scale
    for r in &out {
        let product = f64::from(r.parts.rrf)
            * r.parts.activation
            * r.parts.feedback
            * r.parts.quality
            * r.parts.supersede;
        assert!((r.parts.final_score - product).abs() < 1e-12);
    }
}

#[test]
fn carries_body_and_simhash_for_diversify() {
    let (_d, conn) = open_seeded();
    let cfg = comemory::config::Config::defaults();
    let out = rerank(&conn, &cfg, &[hit("aaaa0002", 1.0)], None).expect("rerank");
    assert_eq!(out[0].body, "second body");
    assert_eq!(out[0].simhash, 2);
    assert_eq!(out[0].source, Source::Lexical);
}

/// Seed a superseder of `aaaa0001` created at `created`, live by default.
fn seed_superseder(conn: &rusqlite::Connection, id: &str, created: &str) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, access_count, last_accessed, simhash)
         VALUES (?1, ?1, 'note', 'demo', 'f', 3, 1, 'hs', 'superseder body',
                 ?2, ?2, 'm/s.md', 0, ?2, 9)",
        rusqlite::params![id, created],
    )
    .expect("seed superseder memory");
    conn.execute(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory', ?1, 'memory', 'aaaa0001', 'supersedes', '2026-07-01T00:00:00Z')",
        rusqlite::params![id],
    )
    .expect("seed supersedes edge");
}

/// Rerank `aaaa0001` alone under `as_of_cutoff`.
fn rerank_first(
    conn: &rusqlite::Connection,
    as_of_cutoff: Option<&str>,
) -> comemory::retrieval::rerank::Reranked {
    let cfg = comemory::config::Config::defaults();
    rerank(conn, &cfg, &[hit("aaaa0001", 1.0)], as_of_cutoff)
        .expect("rerank")
        .into_iter()
        .next()
        .expect("one hit")
}

#[test]
fn as_of_before_superseder_creation_unwinds_the_penalty() {
    // AC-2. The fixture's superseder aaaa0002 was created 2026-06-09; as of
    // 2026-04-01 it did not exist yet, so the memory it later replaced must
    // score unpenalized and carry no `superseded_by` annotation.
    let (_d, conn) = open_seeded();
    let scoped = rerank_first(&conn, Some("2026-04-01T00:00:00Z"));
    assert_eq!(
        scoped.superseded_by, None,
        "a superseder created after the cutoff must not annotate"
    );
    assert!(
        (scoped.parts.supersede - 1.0).abs() < 1e-12,
        "penalty must be neutral, got {}",
        scoped.parts.supersede
    );
}

#[test]
fn no_cutoff_keeps_the_penalty_regardless_of_dates() {
    // AC-3 at the unit level: `--until` never reaches this stage, so it
    // passes `None` and the penalty stays present-day — the same result as
    // an unscoped run, whatever the superseder's created_at says.
    let (_d, conn) = open_seeded();
    let unscoped = rerank_first(&conn, None);
    assert_eq!(unscoped.superseded_by.as_deref(), Some("aaaa0002"));
    assert!((unscoped.parts.supersede - 0.2).abs() < 1e-9);
}

#[test]
fn as_of_after_superseder_creation_matches_the_default_run() {
    // AC-4. With the superseder inside the window, as-of changes nothing.
    let (_d, conn) = open_seeded();
    let default_run = rerank_first(&conn, None);
    let scoped = rerank_first(&conn, Some("2026-07-01T00:00:00Z"));
    assert_eq!(scoped.superseded_by, default_run.superseded_by);
    assert!((scoped.parts.supersede - default_run.parts.supersede).abs() < 1e-12);
    assert!((scoped.parts.final_score - default_run.parts.final_score).abs() < 1e-12);
}

#[test]
fn multiple_superseders_always_report_the_earliest() {
    // AC-14. aaaa0001 is superseded by the fixture's aaaa0002 (2026-06-09)
    // plus two hand-seeded rows on either side. The reported superseder is
    // the earliest live one, identically on every call and under a cutoff
    // that still includes it — the query orders before LIMIT 1 rather than
    // letting SQLite pick whichever row it reaches first.
    let (_d, conn) = open_seeded();
    seed_superseder(&conn, "cccc0001", "2026-05-01T00:00:00Z");
    seed_superseder(&conn, "cccc0002", "2026-06-01T00:00:00Z");

    let default_run = rerank_first(&conn, None);
    assert_eq!(
        default_run.superseded_by.as_deref(),
        Some("cccc0001"),
        "earliest live superseder wins"
    );

    let scoped = rerank_first(&conn, Some("2026-05-15T00:00:00Z"));
    assert_eq!(
        scoped.superseded_by.as_deref(),
        Some("cccc0001"),
        "a cutoff that still includes the earliest superseder reports it"
    );

    for _ in 0..5 {
        assert_eq!(
            rerank_first(&conn, None).superseded_by,
            default_run.superseded_by,
            "repeated runs must report the same superseder"
        );
    }
}
