#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/learning/evaluation/dataset_dedup.rs`: the
//! three rules that decide which observation and which verdict survive.

use comemory::domains::learning::evaluation::candidate_identity::parse_ref;
use comemory::domains::learning::evaluation::dataset_dedup::{
    ContradictionInput, DuplicateInput, VerdictOutcome, classify, collapse_duplicates,
    resolve_contradictions,
};

const KEY_A: &str = "activation decay\u{0}k0\u{0}c0\u{0}{}";
const KEY_B: &str = "chunk boundaries\u{0}k0\u{0}c0\u{0}{}";

fn duplicate<'a>(
    dedup_key: &'a str,
    at: &'a str,
    observation_id: &'a str,
    verdicts: u64,
) -> DuplicateInput<'a> {
    DuplicateInput {
        dedup_key,
        at,
        observation_id,
        verdicts,
    }
}

fn contradiction<'a>(
    key: &'a str,
    relevance: u8,
    at: &'a str,
    observation_id: &'a str,
) -> ContradictionInput<'a> {
    ContradictionInput {
        key,
        relevance,
        at,
        observation_id,
    }
}

#[test]
fn the_most_judged_duplicate_survives_even_when_it_is_the_older_one() {
    let rows = [
        duplicate(KEY_A, "2026-09-16T10:00:00Z", "o-1", 2),
        duplicate(KEY_A, "2026-09-18T10:00:00Z", "o-2", 0),
        duplicate(KEY_B, "2026-09-18T10:00:00Z", "o-3", 0),
    ];

    let dropped = collapse_duplicates(&rows);

    assert_eq!(
        dropped.into_iter().collect::<Vec<_>>(),
        vec![1],
        "keeping the newer but unjudged duplicate would throw away every verdict, \
         which is the expensive artifact"
    );
}

#[test]
fn equal_verdict_counts_fall_back_to_the_later_capture_then_the_greater_id() {
    let by_time = [
        duplicate(KEY_A, "2026-09-16T10:00:00Z", "o-1", 1),
        duplicate(KEY_A, "2026-09-18T10:00:00Z", "o-2", 1),
    ];
    assert_eq!(
        collapse_duplicates(&by_time)
            .into_iter()
            .collect::<Vec<_>>(),
        vec![0]
    );

    let by_id = [
        duplicate(KEY_A, "2026-09-18T10:00:00Z", "o-a", 1),
        duplicate(KEY_A, "2026-09-18T10:00:00Z", "o-b", 1),
    ];
    assert_eq!(
        collapse_duplicates(&by_id).into_iter().collect::<Vec<_>>(),
        vec![0],
        "an equal instant is broken by the greater observation id, so the answer \
         never depends on which row was read first"
    );
}

#[test]
fn distinct_keys_are_never_collapsed_and_an_empty_input_drops_nothing() {
    let rows = [
        duplicate(KEY_A, "2026-09-18T10:00:00Z", "o-1", 0),
        duplicate(KEY_B, "2026-09-18T10:00:00Z", "o-2", 0),
    ];
    assert!(collapse_duplicates(&rows).is_empty());
    assert!(collapse_duplicates(&[]).is_empty());
}

#[test]
fn a_revised_verdict_wins_and_the_one_it_revised_is_dropped_whole() {
    let key = "qg-1\u{0}memory:a1:ff\u{0}manual";
    let rows = [
        contradiction(key, 3, "2026-09-16T10:00:00Z", "o-1"),
        contradiction(key, 0, "2026-09-18T10:00:00Z", "o-2"),
    ];

    let dropped = resolve_contradictions(&rows);

    assert_eq!(
        dropped.into_iter().collect::<Vec<_>>(),
        vec![0],
        "the later verdict revises the earlier one, matching the table's own \
         re-judgment rule extended across observations"
    );
}

#[test]
fn agreeing_verdicts_are_not_a_contradiction_and_neither_class_contends() {
    let key = "qg-1\u{0}memory:a1:ff\u{0}manual";
    let agree = [
        contradiction(key, 3, "2026-09-16T10:00:00Z", "o-1"),
        contradiction(key, 3, "2026-09-18T10:00:00Z", "o-2"),
    ];
    assert!(
        resolve_contradictions(&agree).is_empty(),
        "two observations recording the same verdict are two valid examples, not a clash"
    );

    let implicit_key = "qg-1\u{0}memory:a1:ff\u{0}implicit";
    let across_classes = [
        contradiction(key, 3, "2026-09-16T10:00:00Z", "o-1"),
        contradiction(implicit_key, 0, "2026-09-18T10:00:00Z", "o-2"),
    ];
    assert!(
        resolve_contradictions(&across_classes).is_empty(),
        "a reviewed verdict and an implicit signal are different evidence classes and \
         live in different files, so they never contend"
    );
}

#[test]
fn an_equal_instant_contradiction_is_broken_by_the_greater_observation_id() {
    let key = "qg-1\u{0}memory:a1:ff\u{0}manual";
    let rows = [
        contradiction(key, 3, "2026-09-18T10:00:00Z", "o-a"),
        contradiction(key, 1, "2026-09-18T10:00:00Z", "o-b"),
    ];

    assert_eq!(
        resolve_contradictions(&rows)
            .into_iter()
            .collect::<Vec<_>>(),
        vec![0]
    );
}

#[test]
fn a_verdict_is_matched_stale_or_a_pool_miss_through_the_contracts_own_matcher() {
    let observed = [
        parse_ref("memory:5a9f19bc:aaaa").expect("parse"),
        parse_ref("code:demo:src/ranking.rs:activation_boost:1111").expect("parse"),
    ];

    let hit = parse_ref("memory:5a9f19bc:aaaa").expect("parse");
    assert_eq!(
        classify(&hit, &observed, "o-1").expect("classify"),
        VerdictOutcome::Matched(0)
    );

    let changed = parse_ref("memory:5a9f19bc:bbbb").expect("parse");
    assert_eq!(
        classify(&changed, &observed, "o-1").expect("classify"),
        VerdictOutcome::Stale,
        "the identity is the same memory at a content version this observation \
         never saw, which is stale and never a match"
    );

    let absent = parse_ref("memory:deadbeef:cccc").expect("parse");
    assert_eq!(
        classify(&absent, &observed, "o-1").expect("classify"),
        VerdictOutcome::RecallMiss,
        "a positive retrieval did not return must never be inserted"
    );

    let reindexed = parse_ref("code:demo:src/ranking.rs:activation_boost:9999").expect("parse");
    assert_eq!(
        classify(&reindexed, &observed, "o-1").expect("classify"),
        VerdictOutcome::Stale,
        "a re-indexed file keeps its (repo, path, symbol) identity and changes its blob OID"
    );

    assert_eq!(
        classify(&hit, &[], "o-1").expect("classify"),
        VerdictOutcome::RecallMiss
    );
}
