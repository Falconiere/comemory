//! Tests for [`comemory::retrieval::router::route`].
//!
//! Covers pure-lexical, pure-vector (empty query), and hybrid (vec + query)
//! paths.

use comemory::config::Config;
use comemory::retrieval::router::{self, Source};
use comemory::store::{connection, fts, vector};
use tempfile::tempdir;

#[path = "../common/vectors.rs"]
mod vectors;

fn seed_memory(conn: &rusqlite::Connection, id: &str, body: &str) {
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES(?1, 'x','note','h', ?2, 't','t','x.md')",
        rusqlite::params![id, body],
    )
    .expect("seed memory");
}

#[test]
fn lexical_path_when_no_vector() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    seed_memory(&conn, "lex1", "advisory lock postgres");
    fts::index_memory(&conn, "lex1", "advisory lock postgres", "").expect("fts");

    let cfg = Config::defaults();
    let hits = router::route(&cfg, &conn, "advisory lock", None, None).expect("route");
    assert!(!hits.is_empty());
    assert_eq!(hits[0].memory_id, "lex1");
    assert_eq!(hits[0].source, Source::Lexical);
}

#[test]
fn pure_vector_path_when_empty_query() {
    // Empty query string → pure-vector path (no hybrid).
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    seed_memory(&conn, "vec1", "irrelevant text");
    let v = vectors::vector("seed", 1024);
    vector::insert_memory(&conn, "vec1", &v).expect("vec");

    let cfg = Config::defaults();
    // Empty query → pure-vector branch.
    let hits = router::route(&cfg, &conn, "", Some(&v), None).expect("route");
    assert!(!hits.is_empty());
    assert_eq!(hits[0].memory_id, "vec1");
    assert_eq!(hits[0].source, Source::Vector);
}

#[test]
fn relaxed_fallback_fires_when_strict_finds_nothing() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, simhash)
         VALUES ('aaaa0001','a','note','d','f',3,1,'h1','the oauth refresh race condition',
                 '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/1.md',1);
         INSERT INTO memory_fts(memory_id, body, tags)
         VALUES ('aaaa0001','the oauth refresh race condition','');",
    )
    .expect("seed");
    let cfg = Config::defaults();
    // strict AND of all three terms fails ('login' absent) → OR tier finds it
    let hits = router::route(&cfg, &conn, "oauth login race", None, None).expect("route");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "aaaa0001");
    assert_eq!(hits[0].source, Source::Lexical);
}

#[test]
fn relaxed_fallback_fires_on_empty_hybrid_result() {
    // Hybrid path with an empty vector table and a strict-miss query: both
    // branches come back empty, so the relaxed OR tier must fire and tag
    // its hits as lexical.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_memory(&conn, "hx1", "the oauth refresh race condition");
    fts::index_memory(&conn, "hx1", "the oauth refresh race condition", "").expect("fts");

    let cfg = Config::defaults();
    let v = vectors::vector("seed", 1024);
    let hits = router::route(&cfg, &conn, "oauth login race", Some(&v), None).expect("route");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "hx1");
    assert_eq!(hits[0].source, Source::Lexical);
}

#[test]
fn hybrid_ann_hit_with_lexical_miss_stays_hybrid() {
    // ANN returns a hit but the strict lexical query misses ('login' is
    // absent): the fused result is non-empty, so the relaxed tier must
    // NOT fire and the source stays Hybrid.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_memory(&conn, "hv1", "the oauth refresh race condition");
    fts::index_memory(&conn, "hv1", "the oauth refresh race condition", "").expect("fts");
    let v = vectors::vector("seed", 1024);
    vector::insert_memory(&conn, "hv1", &v).expect("vec");

    let cfg = Config::defaults();
    let hits = router::route(&cfg, &conn, "oauth login race", Some(&v), None).expect("route");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "hv1");
    assert_eq!(hits[0].source, Source::Hybrid);
}

#[test]
fn single_term_miss_does_not_trigger_relaxed_fallback() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_memory(&conn, "s1", "the oauth refresh race condition");
    fts::index_memory(&conn, "s1", "the oauth refresh race condition", "").expect("fts");

    let cfg = Config::defaults();
    // A plain (non-splittable) single term skips the word-level OR tier
    // (OR of one term is the strict query) and builds an empty subtoken
    // expression, so the whole ladder stays empty.
    let hits = router::route(&cfg, &conn, "kubernetes", None, None).expect("route");
    assert!(
        hits.is_empty(),
        "single absent term must not fall back to OR"
    );
}

#[test]
fn identifier_query_reaches_prose_body_via_subtoken_tier() {
    // Spec scenario: querying the identifier `VecDimMismatch` must find a
    // memory whose body talks about a "dim mismatch" without ever spelling
    // the identifier verbatim. Strict tier misses (phrase over subtokens,
    // non-consecutive in the body), word OR tier is skipped (1 term), so
    // the subtoken OR tier must fire.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    let body = "embedder returned wrong dim mismatch against the vec table";
    seed_memory(&conn, "id1", body);
    fts::index_memory(&conn, "id1", body, "").expect("fts");

    let cfg = Config::defaults();
    let hits = router::route(&cfg, &conn, "VecDimMismatch", None, None).expect("route");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "id1");
    assert_eq!(hits[0].source, Source::Lexical);
}

#[test]
fn subtoken_tier_fires_when_word_or_tier_is_also_empty() {
    // Multi-term query where neither whole word matches but one term's
    // subtokens do: strict → empty, word OR → empty, subtoken OR → hit.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    let body = "embedder returned wrong dim mismatch against the vec table";
    seed_memory(&conn, "id2", body);
    fts::index_memory(&conn, "id2", body, "").expect("fts");

    let cfg = Config::defaults();
    let hits = router::route(&cfg, &conn, "VecDimMismatch kubernetes", None, None).expect("route");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "id2");
    assert_eq!(hits[0].source, Source::Lexical);
}

#[test]
fn hybrid_empty_lexical_leg_retries_ladder_despite_ann_hits() {
    // Regression: a memory reachable only via the subtoken tier used to be
    // suppressed in the hybrid arm whenever the ANN leg returned anything —
    // the fused result was non-empty, so the whole-result fallback never
    // fired. The lexical leg now walks the relaxed ladder before fusion.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    // Target memory: prose mentioning the identifier's parts, FTS only.
    let body = "embedder returned wrong dim mismatch against the vec table";
    seed_memory(&conn, "tgt1", body);
    fts::index_memory(&conn, "tgt1", body, "").expect("fts");
    // Noise memory: carries the vector the query supplies (similarity 1.0,
    // above the threshold) but matches nothing lexically.
    seed_memory(&conn, "noise1", "completely unrelated body");
    let v = vectors::vector("noise-seed", 1024);
    vector::insert_memory(&conn, "noise1", &v).expect("vec");

    let cfg = Config::defaults();
    let hits = router::route(&cfg, &conn, "VecDimMismatch", Some(&v), None).expect("route");
    assert!(
        hits.iter().any(|h| h.memory_id == "tgt1"),
        "subtoken-only memory must survive ANN noise in the hybrid arm: {hits:?}",
    );
    assert!(hits.iter().all(|h| h.source == Source::Hybrid));
}

#[test]
fn vector_hits_below_memory_threshold_are_dropped() {
    // KNN always returns the k nearest rows no matter how far away; the
    // memory_threshold floor (default 0.55 cosine similarity) must drop
    // noise instead of returning it. Two deterministic pseudo-random
    // 1024-d vectors have near-zero cosine similarity.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_memory(&conn, "far1", "irrelevant text");
    let stored = vectors::vector("seed", 1024);
    vector::insert_memory(&conn, "far1", &stored).expect("vec");

    let cfg = Config::defaults();
    let query = vectors::vector("far-away-query", 1024);
    let hits = router::route(&cfg, &conn, "", Some(&query), None).expect("route");
    assert!(
        hits.is_empty(),
        "below-threshold ANN hits must be dropped, got {hits:?}",
    );
}

#[test]
fn hybrid_path_when_both_vector_and_query() {
    // Non-empty query + vector → hybrid RRF path.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    seed_memory(&conn, "h1", "advisory lock postgres migration");
    let v = vectors::vector("seed", 1024);
    vector::insert_memory(&conn, "h1", &v).expect("vec");
    fts::index_memory(&conn, "h1", "advisory lock postgres migration", "").expect("fts");

    let cfg = Config::defaults();
    let hits = router::route(&cfg, &conn, "advisory lock", Some(&v), None).expect("route");
    assert!(!hits.is_empty());
    assert_eq!(hits[0].source, Source::Hybrid);
}
