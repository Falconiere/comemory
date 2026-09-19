#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/learning/evaluation/dataset_manifest.rs`: the
//! two digests that make a repeated export provable, against a real migrated
//! SQLite file.

use comemory::domains::learning::evaluation::dataset_manifest::{
    FilterReport, SplitRatios, SplitReport, dataset_id, snapshot_digest,
};
use comemory::store::candidate_dataset::snapshot;
use comemory::store::candidate_judgments::{NewJudgment, upsert_all};
use comemory::store::candidate_observations::{
    NewCandidate, NewObservation, insert, redact_memory,
};
use comemory::store::connection;
use rusqlite::Connection;

const MEMORY_REF: &str = "memory:5a9f19bc:aaaa";

fn db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn seeded(conn: &Connection) {
    insert(
        conn,
        &NewObservation {
            observation_id: "o-20260918-00000001",
            observation_version: 1,
            query_id: None,
            query: "activation decay",
            source: "find",
            filters_json: "{}",
            retrieval_json: "{}",
            knobs_hash: "k0",
            corpus_digest: "c0",
            decay_frozen: true,
            pool_size: 24,
            page_limit: 12,
            page_offset: 0,
            truncated: false,
            at: "2026-09-18T10:00:00.000000000Z",
        },
        &[NewCandidate {
            pool_position: 1,
            domain: "memory",
            candidate_ref: MEMORY_REF,
            content_version: "aaaa",
            unresolved: false,
            returned_position: Some(1),
            retrieval_score: 0.5,
            rank_in_domain: 1,
            tier: Some(1),
            text: "the recorded passage",
            text_sha256: "d0",
            text_full_bytes: 20,
            text_truncated: false,
            locator_json: "{}",
        }],
    )
    .expect("insert");
}

fn filters() -> FilterReport {
    FilterReport {
        provenance: "manual".into(),
        include_unjudged: false,
        include_holdout: false,
        domains: vec!["code".into(), "document".into(), "memory".into()],
        since: None,
        until: None,
        max_negatives_per_query: 0,
    }
}

fn split() -> SplitReport {
    SplitReport {
        policy: "grouped-hash".into(),
        seed: "comemory-dataset-v1".into(),
        ratios: SplitRatios {
            train: 0.7,
            validation: 0.15,
            holdout: 0.15,
        },
        holdout_repo: None,
        holdout_since: None,
        groups: 1,
        largest_group_rows: 1,
        assignments: Vec::new(),
    }
}

#[test]
fn a_snapshot_digest_is_stable_and_moves_when_a_purge_redacts_a_passage() {
    let (_dir, conn) = db();
    seeded(&conn);
    let before = snapshot_digest(&snapshot(&conn, None, None).expect("snapshot"));
    assert_eq!(
        before,
        snapshot_digest(&snapshot(&conn, None, None).expect("snapshot")),
        "two reads of one unchanged database must digest identically"
    );

    let redacted = redact_memory(&conn, "5a9f19bc").expect("redact");
    assert_eq!(redacted, 1, "the purge must actually have touched the row");
    let after = snapshot_digest(&snapshot(&conn, None, None).expect("snapshot"));

    assert_ne!(
        before, after,
        "a purge blanks a passage in place without touching its observation header, so a \
         digest over headers alone would call two different inputs the same snapshot"
    );
}

#[test]
fn a_new_verdict_changes_the_snapshot_digest() {
    let (_dir, conn) = db();
    seeded(&conn);
    let before = snapshot_digest(&snapshot(&conn, None, None).expect("snapshot"));

    upsert_all(
        &conn,
        &[NewJudgment {
            observation_id: "o-20260918-00000001",
            candidate_ref: MEMORY_REF,
            domain: "memory",
            relevance: 3,
            provenance: "manual",
            at: "2026-09-18T11:00:00.000000000Z",
        }],
    )
    .expect("record");

    assert_ne!(
        before,
        snapshot_digest(&snapshot(&conn, None, None).expect("snapshot"))
    );
}

#[test]
fn a_dataset_id_covers_the_configuration_and_the_snapshot() {
    let baseline = dataset_id(&filters(), &split(), "snap");
    assert!(baseline.starts_with("d-"));
    assert_eq!(baseline.len(), 18);
    assert_eq!(
        baseline,
        dataset_id(&filters(), &split(), "snap"),
        "the id must be a pure function of its inputs"
    );

    assert_ne!(baseline, dataset_id(&filters(), &split(), "other-snapshot"));

    for mutate in [
        (|f: &mut FilterReport| f.include_unjudged = true) as fn(&mut FilterReport),
        |f: &mut FilterReport| f.include_holdout = true,
        |f: &mut FilterReport| f.provenance = "all".into(),
        |f: &mut FilterReport| f.domains = vec!["memory".into()],
        |f: &mut FilterReport| f.since = Some("2026-09-01T00:00:00.000000000Z".into()),
        |f: &mut FilterReport| f.until = Some("2026-09-30T00:00:00.000000000Z".into()),
        |f: &mut FilterReport| f.max_negatives_per_query = 1,
    ] {
        let mut changed = filters();
        mutate(&mut changed);
        assert_ne!(
            baseline,
            dataset_id(&changed, &split(), "snap"),
            "every filtering knob must move the id, or two different datasets share one name: \
             {changed:?}"
        );
    }

    for mutate in [
        (|s: &mut SplitReport| s.seed = "another-seed".into()) as fn(&mut SplitReport),
        |s: &mut SplitReport| s.policy = "other-policy".into(),
        |s: &mut SplitReport| s.holdout_since = Some("2026-09-18T00:00:00.000000000Z".into()),
    ] {
        let mut changed = split();
        mutate(&mut changed);
        assert_ne!(baseline, dataset_id(&filters(), &changed, "snap"));
    }

    let mut reserved = split();
    reserved.holdout_repo = Some("demo".into());
    assert_ne!(baseline, dataset_id(&filters(), &reserved, "snap"));

    let mut rebalanced = split();
    rebalanced.ratios.train = 0.6;
    rebalanced.ratios.validation = 0.25;
    assert_ne!(baseline, dataset_id(&filters(), &rebalanced, "snap"));

    let mut reported = split();
    reported.groups = 42;
    assert_eq!(
        baseline,
        dataset_id(&filters(), &reported, "snap"),
        "a realized count is an observation about the data, not a configuration input"
    );
}

#[test]
fn an_empty_window_digests_to_a_stable_value() {
    let (_dir, conn) = db();
    let empty = snapshot_digest(&snapshot(&conn, None, None).expect("snapshot"));
    assert_eq!(empty.len(), 64);
    assert_eq!(
        empty,
        snapshot_digest(&snapshot(&conn, None, None).expect("snapshot"))
    );
}
