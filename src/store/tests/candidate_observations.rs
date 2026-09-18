#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/candidate_observations.rs`, against a real
//! migrated SQLite file.
//!
//! Three behaviours carry the contract and are pinned here: an observation is
//! written all-or-nothing, a purge redacts a passage without disturbing the
//! recorded pool, and the retention sweep never evicts a judged observation.

use comemory::store::candidate_judgments::{NewJudgment, upsert_all};
use comemory::store::candidate_observations::{
    NewCandidate, NewObservation, StoredCandidate, evict_unjudged_before, fetch, insert,
    redact_memory,
};
use comemory::store::connection;

fn seed_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn header<'a>(id: &'a str, at: &'a str) -> NewObservation<'a> {
    NewObservation {
        observation_id: id,
        observation_version: 1,
        query_id: Some("q-20260918-aaaaaaaa"),
        query: "frontmatter contract",
        source: "find",
        filters_json: r#"{"domains":["memory"]}"#,
        retrieval_json: r#"{"binary_version":"0.36.0"}"#,
        knobs_hash: "k0",
        corpus_digest: "c0",
        decay_frozen: false,
        pool_size: 24,
        page_limit: 12,
        page_offset: 0,
        truncated: false,
        at,
    }
}

/// The candidates of `observation_id`, which must exist.
fn candidates_of(conn: &rusqlite::Connection, observation_id: &str) -> Vec<StoredCandidate> {
    fetch(conn, observation_id)
        .expect("fetch")
        .expect("the observation exists")
        .1
}

fn memory_candidate<'a>(position: i64, reference: &'a str, body: &'a str) -> NewCandidate<'a> {
    NewCandidate {
        pool_position: position,
        domain: "memory",
        candidate_ref: reference,
        content_version: reference.rsplit(':').next().unwrap_or_default(),
        unresolved: false,
        returned_position: Some(position),
        retrieval_score: 0.5,
        rank_in_domain: position,
        tier: Some(1),
        text: body,
        text_sha256: "d0",
        text_full_bytes: body.len() as i64,
        text_truncated: false,
        locator_json: r#"{"title":"t"}"#,
    }
}

#[test]
fn an_observation_round_trips_its_header_and_its_pool_order() {
    let (_d, conn) = seed_db();
    let a = "memory:aaaa0001:hash1";
    let b = "memory:bbbb0002:hash2";
    insert(
        &conn,
        &header("o-20260918-00000001", "2026-09-18T10:00:00.000000000Z"),
        &[
            memory_candidate(1, a, "first body"),
            NewCandidate {
                returned_position: None,
                ..memory_candidate(2, b, "second body")
            },
        ],
    )
    .expect("insert");

    let (stored, rows) = fetch(&conn, "o-20260918-00000001")
        .expect("fetch")
        .expect("the observation exists");
    assert_eq!(stored.observation_version, 1);
    assert_eq!(
        stored.candidate_count, 2,
        "the header counts what was written"
    );
    assert_eq!(stored.query, "frontmatter contract");
    assert_eq!(stored.knobs_hash, "k0");
    assert_eq!(stored.corpus_digest, "c0");
    assert!(!stored.truncated);
    let at: String = conn
        .query_row(
            "SELECT at FROM candidate_query_observations WHERE observation_id = ?1",
            ["o-20260918-00000001"],
            |r| r.get(0),
        )
        .expect("read at");
    assert_eq!(at, "2026-09-18T10:00:00.000000000Z");

    assert_eq!(
        rows.iter()
            .map(|r| (
                r.pool_position,
                r.candidate_ref.as_str(),
                r.returned_position
            ))
            .collect::<Vec<_>>(),
        vec![(1, a, Some(1)), (2, b, None)],
        "the pool is returned in retrieval's own order, and a candidate below \
         the page cut keeps its pool position with no returned position"
    );
}

#[test]
fn an_unknown_observation_has_no_header_and_no_candidates() {
    let (_d, conn) = seed_db();
    assert!(
        fetch(&conn, "o-20260918-deadbeef")
            .expect("fetch")
            .is_none(),
        "an unknown observation reads as absent, never as an empty one"
    );
}

#[test]
fn a_failed_candidate_write_rolls_the_whole_observation_back() {
    let (_d, conn) = seed_db();
    // Two candidates at the same pool position violate the composite primary
    // key, so the second insert fails after the header already went in.
    let err = insert(
        &conn,
        &header("o-20260918-00000002", "2026-09-18T10:00:00.000000000Z"),
        &[
            memory_candidate(1, "memory:aaaa0001:hash1", "first"),
            memory_candidate(1, "memory:bbbb0002:hash2", "second"),
        ],
    )
    .expect_err("a duplicate pool position must fail");
    assert!(
        format!("{err}").to_lowercase().contains("unique"),
        "expected a uniqueness failure, got: {err}"
    );
    assert!(
        fetch(&conn, "o-20260918-00000002")
            .expect("fetch")
            .is_none(),
        "a header must never survive without its candidates"
    );
}

#[test]
fn purging_a_memory_redacts_only_its_own_candidates() {
    let (_d, conn) = seed_db();
    insert(
        &conn,
        &header("o-20260918-00000003", "2026-09-18T10:00:00.000000000Z"),
        &[
            memory_candidate(1, "memory:aaaa0001:hash1", "the purged body"),
            memory_candidate(2, "memory:bbbb0002:hash2", "an untouched body"),
        ],
    )
    .expect("insert");

    assert_eq!(redact_memory(&conn, "aaaa0001").expect("redact"), 1);

    let rows = candidates_of(&conn, "o-20260918-00000003");
    assert_eq!(rows.len(), 2, "redaction must not shrink the recorded pool");
    assert!(
        rows[0].unresolved,
        "the purged candidate is marked unresolved"
    );
    assert_eq!(rows[0].candidate_ref, "memory:aaaa0001:hash1");
    assert_eq!(rows[0].pool_position, 1);
    assert!(
        !rows[1].unresolved,
        "another memory's candidate is untouched"
    );

    let text: String = conn
        .query_row(
            "SELECT text FROM candidate_observations WHERE pool_position = 1",
            [],
            |r| r.get(0),
        )
        .expect("read text");
    assert!(text.is_empty(), "the purged passage must be gone");
}

#[test]
fn redaction_treats_a_memory_id_as_a_literal_prefix() {
    let (_d, conn) = seed_db();
    insert(
        &conn,
        &header("o-20260918-00000004", "2026-09-18T10:00:00.000000000Z"),
        &[memory_candidate(1, "memory:aaaa0001:hash1", "body")],
    )
    .expect("insert");
    // `_` and `%` are LIKE wildcards; the prefix comparison must not treat
    // them as such, or purging one memory would redact unrelated candidates.
    assert_eq!(redact_memory(&conn, "aaaa000_").expect("redact"), 0);
    assert_eq!(redact_memory(&conn, "%").expect("redact"), 0);
}

#[test]
fn the_retention_sweep_evicts_an_unjudged_observation_and_keeps_a_judged_one() {
    let (_d, conn) = seed_db();
    let old = "2026-01-01T00:00:00.000000000Z";
    for id in ["o-20260101-00000001", "o-20260101-00000002"] {
        insert(
            &conn,
            &header(id, old),
            &[memory_candidate(1, "memory:aaaa0001:hash1", "body")],
        )
        .expect("insert");
    }
    upsert_all(
        &conn,
        &[NewJudgment {
            observation_id: "o-20260101-00000002",
            candidate_ref: "memory:aaaa0001:hash1",
            domain: "memory",
            relevance: 3,
            provenance: "manual",
            at: old,
        }],
    )
    .expect("judge");

    let (headers, candidates) =
        evict_unjudged_before(&conn, "2026-06-01T00:00:00.000000000Z").expect("evict");
    assert_eq!((headers, candidates), (1, 1));
    assert!(
        fetch(&conn, "o-20260101-00000001")
            .expect("fetch")
            .is_none()
    );
    assert_eq!(
        candidates_of(&conn, "o-20260101-00000002").len(),
        1,
        "a judged observation and its candidates are retained however old they \
         are, or the verdict loses the passage it was made against"
    );
}

#[test]
fn the_retention_cutoff_is_exclusive() {
    let (_d, conn) = seed_db();
    let cutoff = "2026-01-01T00:00:00.000000000Z";
    insert(
        &conn,
        &header("o-20260101-00000003", cutoff),
        &[memory_candidate(1, "memory:aaaa0001:hash1", "body")],
    )
    .expect("insert");
    assert_eq!(
        evict_unjudged_before(&conn, cutoff).expect("evict"),
        (0, 0),
        "a row exactly at the cutoff survives, matching gc_learning::evict_before"
    );
}
