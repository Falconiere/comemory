#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `output::explain::parts_of` over real `ScoreParts` / `CodeScoreParts` /
//! `DocParts` values — the same types `retrieval::rerank` and
//! `retrieval::code_rerank` hand to `fuse_domains` (console-api spec §3).
//!
//! The contract under test is the share partition: `share` is a
//! log-magnitude split across the multiplicative priors, leg signals are
//! informational (`share == 0`), and the priors' shares sum to 1.

use comemory::output::explain::{ExplainPart, parts_of};
use comemory::retrieval::code_rerank::CodeScoreParts;
use comemory::retrieval::rerank::ScoreParts;
use comemory::retrieval::score::LegScores;
use comemory::retrieval::unified::fuse_domains::{DocParts, HitParts};

/// The prior names each domain reports — anything else in the strip is a
/// leg signal and stays out of the partition.
const MEMORY_PRIORS: &[&str] = &["activation", "feedback", "quality", "supersede", "pagerank"];
const CODE_PRIORS: &[&str] = &["pagerank", "activation", "affinity", "feedback"];

fn memory_parts(legs: LegScores) -> HitParts {
    HitParts::Memory(Box::new(ScoreParts {
        rrf: 1.0,
        activation: 1.4,
        feedback: 0.8,
        quality: 1.2,
        supersede: 1.0,
        rank: 1.14,
        final_score: 1.4 * 0.8 * 1.2 * 1.0 * 1.14,
        legs,
    }))
}

fn code_parts(legs: LegScores) -> HitParts {
    HitParts::Code(Box::new(CodeScoreParts {
        relevance: 1.0,
        rank: 1.3,
        activation: 0.9,
        affinity: 1.05,
        feedback: 1.0,
        final_score: 1.3 * 0.9 * 1.05,
        legs,
    }))
}

fn named<'a>(strip: &'a [ExplainPart], name: &str) -> &'a ExplainPart {
    strip
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no `{name}` part in {:?}", names(strip)))
}

fn names(strip: &[ExplainPart]) -> Vec<String> {
    strip.iter().map(|p| p.name.clone()).collect()
}

fn prior_share_sum(strip: &[ExplainPart], priors: &[&str]) -> f64 {
    priors.iter().map(|n| named(strip, n).share).sum()
}

#[test]
fn memory_strip_carries_every_prior_and_only_the_legs_that_fired() {
    let strip = parts_of(&memory_parts(LegScores {
        bm25: Some(3.25),
        ann: None,
    }));

    assert_eq!(
        names(&strip),
        vec![
            "bm25".to_string(),
            "activation".into(),
            "feedback".into(),
            "quality".into(),
            "supersede".into(),
            "pagerank".into(),
        ],
        "legs first, then the priors in rerank order; an absent ann leg is omitted"
    );
    assert_eq!(named(&strip, "bm25").note, "score 3.2500");
    assert_eq!(named(&strip, "activation").value, 1.4);
    assert_eq!(named(&strip, "activation").note, "×1.400");
}

#[test]
fn an_ann_leg_is_reported_as_a_cosine() {
    let strip = parts_of(&memory_parts(LegScores {
        bm25: Some(1.5),
        ann: Some(0.8125),
    }));
    let ann = named(&strip, "ann");
    assert_eq!(ann.note, "cosine 0.8125");
    assert_eq!(ann.value, f64::from(0.8125_f32));
    assert_eq!(ann.share, 0.0, "a leg signal never enters the partition");
}

#[test]
fn memory_prior_shares_sum_to_one_and_a_neutral_factor_takes_none_of_it() {
    let strip = parts_of(&memory_parts(LegScores {
        bm25: Some(2.0),
        ann: Some(0.7),
    }));

    let sum = prior_share_sum(&strip, MEMORY_PRIORS);
    assert!(
        (sum - 1.0).abs() < 1e-9,
        "prior shares must partition the rerank, got {sum}"
    );
    assert_eq!(
        named(&strip, "supersede").share,
        0.0,
        "a factor of exactly 1.0 did not move the score"
    );
    assert_eq!(
        named(&strip, "bm25").share + named(&strip, "ann").share,
        0.0,
        "leg signals are informational"
    );
    assert!(
        named(&strip, "activation").share > named(&strip, "pagerank").share,
        "1.4 moved the score further from neutral than 1.14 did"
    );
}

#[test]
fn a_below_one_prior_earns_share_by_magnitude_not_by_direction() {
    // feedback 0.8 (a demotion) and quality 1.25 (a promotion) are the same
    // distance from 1.0 in log space, so they must split the partition
    // evenly when they are the only two non-neutral factors.
    let strip = parts_of(&HitParts::Memory(Box::new(ScoreParts {
        rrf: 1.0,
        activation: 1.0,
        feedback: 0.8,
        quality: 1.25,
        supersede: 1.0,
        rank: 1.0,
        final_score: 1.0,
        legs: LegScores::none(),
    })));

    let feedback = named(&strip, "feedback").share;
    let quality = named(&strip, "quality").share;
    assert!(
        (feedback - quality).abs() < 1e-9,
        "|ln 0.8| == |ln 1.25|, so {feedback} and {quality} must match"
    );
    assert!((feedback + quality - 1.0).abs() < 1e-9);
}

#[test]
fn an_all_neutral_rerank_splits_the_partition_evenly() {
    let strip = parts_of(&HitParts::Memory(Box::new(ScoreParts {
        rrf: 1.0,
        activation: 1.0,
        feedback: 1.0,
        quality: 1.0,
        supersede: 1.0,
        rank: 1.0,
        final_score: 1.0,
        legs: LegScores::none(),
    })));

    for name in MEMORY_PRIORS {
        assert!(
            (named(&strip, name).share - 0.2).abs() < 1e-9,
            "{name} should take 1/5 of a zero-magnitude partition"
        );
    }
    assert!((prior_share_sum(&strip, MEMORY_PRIORS) - 1.0).abs() < 1e-9);
}

#[test]
fn code_strip_reports_its_own_four_priors_and_partitions_them() {
    let strip = parts_of(&code_parts(LegScores {
        bm25: Some(4.0),
        ann: Some(0.61),
    }));

    assert_eq!(
        names(&strip),
        vec![
            "bm25".to_string(),
            "ann".into(),
            "pagerank".into(),
            "activation".into(),
            "affinity".into(),
            "feedback".into(),
        ]
    );
    let sum = prior_share_sum(&strip, CODE_PRIORS);
    assert!((sum - 1.0).abs() < 1e-9, "got {sum}");
    assert_eq!(named(&strip, "feedback").share, 0.0);
}

#[test]
fn a_document_hit_reports_its_bm25_position_and_no_priors() {
    let strip = parts_of(&HitParts::Document(DocParts { bm25_rank: 3 }));

    assert_eq!(names(&strip), vec!["bm25_rank".to_string()]);
    assert_eq!(strip[0].value, 3.0);
    assert_eq!(strip[0].note, "rank 3");
    assert_eq!(
        strip[0].share, 0.0,
        "the document leg has no rerank stage, so nothing to partition"
    );
}
