#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/candidate_judgments.rs`, against a real migrated
//! SQLite file.
//!
//! The declared `CHECK`s are the point: a grade outside `0..=MAX_RELEVANCE`, a
//! provenance outside the #130 vocabulary, or an unknown domain must be
//! refused by the database itself, not only by the caller that happens to
//! write today.

use comemory::store::candidate_judgments::{NewJudgment, fetch_for_observation, upsert_all};
use comemory::store::connection;

fn seed_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn judgment(reference: &str, relevance: i64) -> NewJudgment<'_> {
    NewJudgment {
        observation_id: "o-20260918-00000001",
        candidate_ref: reference,
        domain: "memory",
        relevance,
        provenance: "manual",
        at: "2026-09-18T10:00:00.000000000Z",
    }
}

#[test]
fn verdicts_round_trip_keyed_by_candidate_reference() {
    let (_d, conn) = seed_db();
    let written = upsert_all(
        &conn,
        &[
            judgment("memory:aaaa0001:hash1", 3),
            NewJudgment {
                domain: "document",
                ..judgment("document:d1:guides/chunking.md:rev1:0", 1)
            },
        ],
    )
    .expect("upsert");
    assert_eq!(written, 2);

    let stored = fetch_for_observation(&conn, "o-20260918-00000001").expect("fetch");
    assert_eq!(stored.get("memory:aaaa0001:hash1"), Some(&3));
    assert_eq!(
        stored.get("document:d1:guides/chunking.md:rev1:0"),
        Some(&1)
    );
}

#[test]
fn re_judging_the_same_candidate_replaces_the_verdict() {
    let (_d, conn) = seed_db();
    upsert_all(&conn, &[judgment("memory:aaaa0001:hash1", 3)]).expect("first");
    upsert_all(&conn, &[judgment("memory:aaaa0001:hash1", 0)]).expect("second");

    let stored = fetch_for_observation(&conn, "o-20260918-00000001").expect("fetch");
    assert_eq!(
        stored.len(),
        1,
        "a re-judgment replaces rather than accumulates"
    );
    assert_eq!(
        stored.get("memory:aaaa0001:hash1"),
        Some(&0),
        "0 is an explicit `reviewed and not relevant`, not an absent verdict"
    );
}

#[test]
fn an_empty_batch_writes_nothing() {
    let (_d, conn) = seed_db();
    assert_eq!(upsert_all(&conn, &[]).expect("upsert"), 0);
    assert!(
        fetch_for_observation(&conn, "o-20260918-00000001")
            .expect("fetch")
            .is_empty()
    );
}

#[test]
fn the_database_refuses_a_grade_a_provenance_or_a_domain_outside_its_vocabulary() {
    let (_d, conn) = seed_db();
    for bad in [
        judgment("memory:aaaa0001:hash1", 4),
        judgment("memory:aaaa0001:hash1", -1),
        NewJudgment {
            provenance: "explicit",
            ..judgment("memory:aaaa0001:hash1", 2)
        },
        NewJudgment {
            domain: "memories",
            ..judgment("memory:aaaa0001:hash1", 2)
        },
    ] {
        let err = upsert_all(&conn, &[bad]).expect_err("the CHECK must refuse it");
        assert!(
            format!("{err}").to_lowercase().contains("constraint"),
            "expected a CHECK constraint failure, got: {err}"
        );
    }
    assert!(
        fetch_for_observation(&conn, "o-20260918-00000001")
            .expect("fetch")
            .is_empty()
    );
}

#[test]
fn a_rejected_row_rolls_the_whole_batch_back() {
    let (_d, conn) = seed_db();
    upsert_all(
        &conn,
        &[
            judgment("memory:aaaa0001:hash1", 3),
            judgment("memory:bbbb0002:hash2", 9),
        ],
    )
    .expect_err("the second row is out of range");
    assert!(
        fetch_for_observation(&conn, "o-20260918-00000001")
            .expect("fetch")
            .is_empty(),
        "a batch is one unit: the valid row must not survive the invalid one"
    );
}
