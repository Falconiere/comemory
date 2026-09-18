#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/candidate_dataset.rs`, against a real migrated
//! SQLite file written through the production `candidate_observations` and
//! `candidate_judgments` writers.

use comemory::store::candidate_dataset::snapshot;
use comemory::store::candidate_judgments::{NewJudgment, upsert_all};
use comemory::store::candidate_observations::{NewCandidate, NewObservation, insert};
use comemory::store::connection;

const MEMORY_REF: &str = "memory:a1b2c3d4:ff00";
const DOC_REF: &str = "document:guides/chunking.md:rev1:0";

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
        decay_frozen: true,
        pool_size: 24,
        page_limit: 12,
        page_offset: 0,
        truncated: false,
        at,
    }
}

fn candidate<'a>(position: i64, domain: &'a str, reference: &'a str) -> NewCandidate<'a> {
    NewCandidate {
        pool_position: position,
        domain,
        candidate_ref: reference,
        content_version: reference.rsplit(':').next().unwrap_or_default(),
        unresolved: false,
        returned_position: Some(position),
        retrieval_score: 0.5,
        rank_in_domain: position,
        tier: Some(1),
        text: "the recorded passage",
        text_sha256: "d0",
        text_full_bytes: 20,
        text_truncated: false,
        locator_json: r#"{"title":"A DISPLAY TITLE","repo":null,"path":null,
            "line_range":null,"heading_path":null,"symbol_id":null}"#,
    }
}

/// Three observations a day apart, each with two candidates; the middle one
/// carries a verdict.
fn seeded() -> (tempfile::TempDir, rusqlite::Connection) {
    let (dir, conn) = seed_db();
    for (id, at) in [
        ("o-20260916-00000001", "2026-09-16T10:00:00.000000000Z"),
        ("o-20260917-00000002", "2026-09-17T10:00:00.000000000Z"),
        ("o-20260918-00000003", "2026-09-18T10:00:00.000000000Z"),
    ] {
        insert(
            &conn,
            &header(id, at),
            &[
                candidate(1, "memory", MEMORY_REF),
                candidate(2, "document", DOC_REF),
            ],
        )
        .expect("insert");
    }
    upsert_all(
        &conn,
        &[NewJudgment {
            observation_id: "o-20260917-00000002",
            candidate_ref: MEMORY_REF,
            domain: "memory",
            relevance: 3,
            provenance: "manual",
            at: "2026-09-17T11:00:00.000000000Z",
        }],
    )
    .expect("upsert");
    (dir, conn)
}

#[test]
fn an_unbounded_snapshot_returns_every_row_in_a_fixed_order() {
    let (_dir, conn) = seeded();

    let snap = snapshot(&conn, None, None).expect("snapshot");

    assert_eq!(
        snap.observations
            .iter()
            .map(|o| o.observation_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "o-20260916-00000001",
            "o-20260917-00000002",
            "o-20260918-00000003"
        ],
        "observations must be ascending by (at, observation_id)"
    );
    assert_eq!(snap.candidates.len(), 6, "two candidates per observation");
    let order: Vec<(&str, i64)> = snap
        .candidates
        .iter()
        .map(|c| (c.observation_id.as_str(), c.pool_position))
        .collect();
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(
        order, sorted,
        "candidates must be ascending by (observation_id, pool_position)"
    );
    assert_eq!(snap.judgments.len(), 1);
    assert_eq!(snap.judgments[0].relevance, 3);
    assert_eq!(snap.judgments[0].provenance, "manual");
}

#[test]
fn the_window_is_half_open_on_both_sides_and_each_bound_is_optional() {
    let (_dir, conn) = seeded();
    let middle = "2026-09-17T10:00:00.000000000Z";
    let last = "2026-09-18T10:00:00.000000000Z";

    let from_middle = snapshot(&conn, Some(middle), None).expect("since only");
    assert_eq!(
        from_middle.observations.len(),
        2,
        "`since` is inclusive, so the observation at the bound is in"
    );

    let before_last = snapshot(&conn, None, Some(last)).expect("until only");
    assert_eq!(
        before_last.observations.len(),
        2,
        "`until` is exclusive, so the observation at the bound is out"
    );

    let one = snapshot(&conn, Some(middle), Some(last)).expect("both");
    assert_eq!(one.observations.len(), 1);
    assert_eq!(one.observations[0].observation_id, "o-20260917-00000002");
    assert_eq!(
        one.candidates.len(),
        2,
        "a windowed read must not return candidates of observations outside it"
    );
    assert_eq!(
        one.judgments.len(),
        1,
        "the verdict of the windowed observation comes with it"
    );

    let empty = snapshot(&conn, Some(last), Some(last)).expect("empty window");
    assert!(empty.observations.is_empty());
    assert!(empty.candidates.is_empty());
    assert!(empty.judgments.is_empty());
}

#[test]
fn a_verdict_outside_the_window_never_travels_with_an_observation_inside_it() {
    let (_dir, conn) = seeded();
    upsert_all(
        &conn,
        &[NewJudgment {
            observation_id: "o-20260918-00000003",
            candidate_ref: DOC_REF,
            domain: "document",
            relevance: 0,
            provenance: "manual",
            at: "2026-09-18T11:00:00.000000000Z",
        }],
    )
    .expect("upsert");

    let early = snapshot(&conn, None, Some("2026-09-18T00:00:00.000000000Z")).expect("snapshot");

    assert_eq!(early.observations.len(), 2);
    assert_eq!(
        early.judgments.len(),
        1,
        "the second verdict belongs to an observation this window excludes"
    );
    assert_eq!(early.judgments[0].observation_id, "o-20260917-00000002");
}

#[test]
fn the_projection_carries_the_passage_and_never_the_locator() {
    let (_dir, conn) = seeded();

    let snap = snapshot(&conn, None, None).expect("snapshot");

    let memory = snap
        .candidates
        .iter()
        .find(|c| c.candidate_ref == MEMORY_REF)
        .expect("the memory candidate");
    assert_eq!(memory.text, "the recorded passage");
    assert_eq!(memory.text_sha256, "d0");
    assert_eq!(memory.content_version, "ff00");
    assert!(!memory.unresolved);
    assert_eq!(memory.returned_position, Some(1));
    assert_eq!(memory.tier, Some(1));
    assert!(snap.observations.iter().all(|o| o.decay_frozen));
    assert_eq!(snap.observations[0].candidate_count, 2);

    // The display title is in the database and is reachable by a direct read,
    // so this assertion is about the projection rather than about the fixture.
    let stored_title: String = conn
        .query_row(
            "SELECT locator_json FROM candidate_observations WHERE candidate_ref = ?1 LIMIT 1",
            [MEMORY_REF],
            |r| r.get(0),
        )
        .expect("the locator is stored");
    assert!(stored_title.contains("A DISPLAY TITLE"));
    // The module's prose explains why the locator is excluded, so the check is
    // against its CODE: every comment line is stripped before the scan, and
    // adding the column back to the projection or to `DatasetCandidate` fails
    // here immediately.
    let module = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/store/candidate_dataset.rs"
    ))
    .expect("read the module");
    let code: String = module
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        code.contains("candidate_ref"),
        "the comment-stripped scan must still see the module's real code"
    );
    assert!(
        !code.contains("locator_json"),
        "the dataset projection must never name the display-only locator column"
    );
}

#[test]
fn an_unresolved_candidate_is_returned_and_flagged_rather_than_hidden() {
    let (_dir, conn) = seed_db();
    let mut redacted = candidate(1, "memory", MEMORY_REF);
    redacted.unresolved = true;
    redacted.content_version = "";
    redacted.text = "";
    redacted.text_full_bytes = 0;
    insert(
        &conn,
        &header("o-20260918-0000000a", "2026-09-18T10:00:00.000000000Z"),
        &[redacted],
    )
    .expect("insert");

    let snap = snapshot(&conn, None, None).expect("snapshot");

    assert_eq!(snap.candidates.len(), 1);
    assert!(
        snap.candidates[0].unresolved,
        "the store reports the flag; deciding what to do with it is the domain's"
    );
    assert!(snap.candidates[0].text.is_empty());
}
