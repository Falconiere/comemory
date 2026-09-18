#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for the benchmark metrics, against hand-computed values: the whole
//! point of pool recall is that it can be 1.0 while recall@k is 0.0, and the
//! whole point of stating judgment coverage is that an unjudged page candidate
//! is distinguishable from a judged-zero one.

use comemory::domains::learning::evaluation::benchmark_metrics::{
    MatchedJudgment, TaskMetrics, paired_delta, score, summarize,
};
use comemory::domains::learning::evaluation::candidate_identity::CandidateDomain;

/// One judgment that matched the given 1-based pool positions.
fn judged(relevance: u8, domain: CandidateDomain, positions: &[usize]) -> MatchedJudgment {
    MatchedJudgment {
        relevance,
        domain,
        pool_positions: positions.to_vec(),
    }
}

/// The identity ordering of a pool of `n` candidates.
fn pool_order(n: usize) -> Vec<usize> {
    (1..=n).collect()
}

#[test]
fn a_relevant_candidate_below_the_cut_is_full_pool_recall_and_zero_recall_at_k() {
    // The relevant memory sits at pool position 4; the page is the first 1.
    let judgments = vec![judged(3, CandidateDomain::Memory, &[4])];
    let m = score(&judgments, &pool_order(6), 1, None);
    assert_eq!(m.pool_recall, 1.0, "retrieval DID produce it");
    assert_eq!(m.recall_at_k, 0.0, "but not within the cut");
    assert_eq!(m.mrr, 0.0);
    assert_eq!(m.ndcg_at_k, 0.0);
    assert_eq!(m.judged_in_page, 0);
    assert_eq!(m.unjudged_in_page, 1);
}

#[test]
fn a_judgment_that_matched_nothing_lowers_pool_recall_without_growing_the_pool() {
    let judgments = vec![
        judged(3, CandidateDomain::Memory, &[1]),
        judged(3, CandidateDomain::Memory, &[]),
    ];
    let m = score(&judgments, &pool_order(3), 3, None);
    assert_eq!(
        m.pool_recall, 0.5,
        "one of two positives was never returned"
    );
    assert_eq!(m.recall_at_k, 0.5, "a missing positive is never inserted");
    assert_eq!(m.mrr, 1.0);
}

#[test]
fn one_judgment_matching_several_candidates_counts_once() {
    // A document judgment keyed on a path can match more than one pooled
    // candidate; counting candidates would make recall exceed its denominator.
    let judgments = vec![judged(2, CandidateDomain::Document, &[1, 2, 3])];
    let m = score(&judgments, &pool_order(3), 3, None);
    assert_eq!(m.pool_recall, 1.0);
    assert_eq!(m.recall_at_k, 1.0);
    assert_eq!(m.judged_in_page, 3, "all three page entries are judged");
}

#[test]
fn an_empty_positive_set_scores_zero_rather_than_one() {
    // The existing eval convention: a task with no relevant judgment carries no
    // signal, so it must not inflate an average with a free 1.0.
    let judgments = vec![judged(0, CandidateDomain::Memory, &[1])];
    let m = score(&judgments, &pool_order(2), 2, None);
    assert_eq!(m.pool_recall, 0.0);
    assert_eq!(m.recall_at_k, 0.0);
    assert_eq!(m.ndcg_at_k, 0.0, "IDCG is zero, so nDCG is zero");
    assert_eq!(m.judged_in_page, 1, "a judged-0 candidate IS judged");
    assert_eq!(m.unjudged_in_page, 1);
    assert_eq!(m.judged_page_fraction, 0.5);
}

#[test]
fn ndcg_matches_the_hand_computed_value_for_a_graded_page() {
    // Page = positions 1,2. Grades 1 then 3.
    // DCG  = (2^1-1)/log2(2) + (2^3-1)/log2(3) = 1 + 7/1.5849625 = 5.415037
    // IDCG = (2^3-1)/log2(2) + (2^1-1)/log2(3) = 7 + 1/1.5849625 = 7.630930
    let judgments = vec![
        judged(1, CandidateDomain::Memory, &[1]),
        judged(3, CandidateDomain::Memory, &[2]),
    ];
    let m = score(&judgments, &pool_order(2), 2, None);
    let expected = (1.0 + 7.0 / 3.0_f64.log2()) / (7.0 + 1.0 / 3.0_f64.log2());
    assert!(
        (m.ndcg_at_k - expected).abs() < 1e-12,
        "ndcg {} should be {expected}",
        m.ndcg_at_k
    );
    assert!(m.ndcg_at_k < 1.0, "the better result is ranked second");
}

#[test]
fn a_perfect_ordering_scores_one() {
    let judgments = vec![
        judged(3, CandidateDomain::Memory, &[1]),
        judged(1, CandidateDomain::Memory, &[2]),
    ];
    let m = score(&judgments, &pool_order(2), 2, None);
    assert_eq!(m.ndcg_at_k, 1.0);
    assert_eq!(m.recall_at_k, 1.0);
    assert_eq!(m.mrr, 1.0);
}

#[test]
fn reordering_the_arm_changes_recall_and_mrr_over_the_same_pool() {
    let judgments = vec![judged(3, CandidateDomain::Memory, &[3])];
    let baseline = score(&judgments, &pool_order(3), 1, None);
    let reranked = score(&judgments, &[3, 1, 2], 1, None);
    assert_eq!(baseline.recall_at_k, 0.0);
    assert_eq!(reranked.recall_at_k, 1.0, "the same pool, a better order");
    assert_eq!(reranked.mrr, 1.0);
    assert_eq!(
        baseline.pool_recall, reranked.pool_recall,
        "pool recall is retrieval's ceiling and no arm can move it"
    );
}

#[test]
fn a_domain_filter_scores_only_that_corpus_judgments() {
    let judgments = vec![
        judged(3, CandidateDomain::Memory, &[1]),
        judged(3, CandidateDomain::Code, &[5]),
    ];
    let all = score(&judgments, &pool_order(6), 2, None);
    let memory = score(&judgments, &pool_order(6), 2, Some(CandidateDomain::Memory));
    let code = score(&judgments, &pool_order(6), 2, Some(CandidateDomain::Code));
    assert_eq!(
        all.recall_at_k, 0.5,
        "one of the two positives is in the page"
    );
    assert_eq!(memory.recall_at_k, 1.0);
    assert_eq!(code.recall_at_k, 0.0);
    assert_eq!(code.pool_recall, 1.0, "the code hit IS in the pool, at 5");
}

#[test]
fn a_page_shorter_than_k_is_scored_over_what_exists() {
    let judgments = vec![judged(3, CandidateDomain::Memory, &[1])];
    let m = score(&judgments, &pool_order(1), 10, None);
    assert_eq!(m.recall_at_k, 1.0);
    assert_eq!(m.unjudged_in_page, 0);
    assert_eq!(m.judged_page_fraction, 1.0);
}

#[test]
fn an_empty_pool_scores_zero_without_dividing_by_zero() {
    let judgments = vec![judged(3, CandidateDomain::Memory, &[])];
    let m = score(&judgments, &[], 3, None);
    assert_eq!(m.pool_recall, 0.0);
    assert_eq!(m.judged_page_fraction, 0.0);
    assert_eq!(m.unjudged_in_page, 0);
}

#[test]
fn the_summary_means_over_judged_tasks_and_reports_both_counts() {
    let per_task = vec![
        TaskMetrics {
            pool_recall: 1.0,
            recall_at_k: 1.0,
            mrr: 1.0,
            ndcg_at_k: 1.0,
            judged_in_page: 1,
            unjudged_in_page: 1,
            judged_page_fraction: 0.5,
        },
        TaskMetrics {
            pool_recall: 0.0,
            recall_at_k: 0.0,
            mrr: 0.0,
            ndcg_at_k: 0.0,
            judged_in_page: 0,
            unjudged_in_page: 2,
            judged_page_fraction: 0.0,
        },
    ];
    let both = summarize(&per_task, &[true, true], 7);
    assert_eq!(both.tasks, 2);
    assert_eq!(both.judged_tasks, 2);
    assert_eq!(both.recall_at_k, 0.5);

    let one = summarize(&per_task, &[true, false], 7);
    assert_eq!(one.tasks, 2, "the unjudged task is still reported");
    assert_eq!(one.judged_tasks, 1);
    assert_eq!(one.recall_at_k, 1.0, "means skip the task with no signal");
    assert_eq!(
        one.judged_in_page, 1,
        "coverage counts every scored page, judged task or not"
    );
    assert_eq!(one.unjudged_in_page, 3);
    assert_eq!(one.judged_page_fraction, 0.25);
}

#[test]
fn the_summary_is_reproducible_for_one_seed_and_moves_with_another() {
    let per_task: Vec<TaskMetrics> = [0.2, 0.4, 0.9, 0.5, 0.1]
        .into_iter()
        .map(|v| TaskMetrics {
            pool_recall: v,
            recall_at_k: v,
            mrr: v,
            ndcg_at_k: v,
            ..TaskMetrics::default()
        })
        .collect();
    let judged = vec![true; per_task.len()];
    let a = summarize(&per_task, &judged, 11);
    let again = summarize(&per_task, &judged, 11);
    assert_eq!(
        a.recall_at_k_ci, again.recall_at_k_ci,
        "one seed, one interval"
    );
    let b = summarize(&per_task, &judged, 12);
    assert_ne!(
        a.recall_at_k_ci, b.recall_at_k_ci,
        "the seed drives the resample"
    );
    assert!(a.recall_at_k_ci.0 <= a.recall_at_k && a.recall_at_k <= a.recall_at_k_ci.1);
}

#[test]
fn a_paired_delta_measures_per_task_differences_not_two_independent_means() {
    let baseline = [0.10, 0.20, 0.30, 0.40];
    let arm = [0.20, 0.30, 0.40, 0.50];
    let delta = paired_delta("ndcg_at_k", &arm, &baseline, 3);
    assert_eq!(delta.metric, "ndcg_at_k");
    assert_eq!(delta.tasks, 4);
    assert!((delta.mean_delta - 0.1).abs() < 1e-12);
    assert!(
        delta.ci.0 > 0.0,
        "every task improved by the same amount, so the interval must exclude zero: {:?}",
        delta.ci
    );
}

#[test]
fn a_paired_delta_over_a_wash_straddles_zero() {
    let baseline = [0.10, 0.90, 0.20, 0.80];
    let arm = [0.90, 0.10, 0.80, 0.20];
    let delta = paired_delta("ndcg_at_k", &arm, &baseline, 3);
    assert!((delta.mean_delta).abs() < 1e-12);
    assert!(
        delta.ci.0 < 0.0 && delta.ci.1 > 0.0,
        "an arm that wins and loses equally must not read as an improvement: {:?}",
        delta.ci
    );
}
