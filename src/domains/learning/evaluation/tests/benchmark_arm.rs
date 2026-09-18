#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for benchmark arms: an arm reorders one captured snapshot and
//! never re-runs retrieval, so its ordering rule has to be total, and a stale
//! or malformed scores file has to be refused rather than half-applied.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;

use comemory::domains::learning::evaluation::benchmark_arm::{
    ArmScores, BASELINE_ARM, baseline_order, scored_order,
};
use tempfile::TempDir;

/// Write `json` into a tempdir and return its path plus the guard.
fn write_scores(json: &str) -> (PathBuf, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("scores.json");
    let mut file = std::fs::File::create(&path).expect("create");
    file.write_all(json.as_bytes()).expect("write");
    (path, tmp)
}

/// Four candidate references, standing for one captured pool.
fn refs() -> Vec<String> {
    (1..=4)
        .map(|i| format!("memory:aaaa000{i}:hash{i}"))
        .collect()
}

#[test]
fn the_baseline_order_is_the_pool_exactly_as_retrieval_produced_it() {
    let order = baseline_order(3);
    assert_eq!(order.order, vec![1, 2, 3]);
    assert_eq!(order.scored_fraction, 1.0);
}

#[test]
fn a_scored_arm_sorts_by_score_and_breaks_ties_on_pool_position() {
    let refs = refs();
    let mut scores = HashMap::new();
    scores.insert(refs[0].clone(), 1.0);
    scores.insert(refs[1].clone(), 5.0);
    scores.insert(refs[2].clone(), 5.0);
    scores.insert(refs[3].clone(), 2.0);
    let order = scored_order(&refs, &scores);
    assert_eq!(
        order.order,
        vec![2, 3, 4, 1],
        "equal scores keep the lower pool position first, so the order is total"
    );
    assert_eq!(order.scored_fraction, 1.0);
}

#[test]
fn unscored_candidates_keep_their_pool_order_and_follow_the_scored_ones() {
    let refs = refs();
    let mut scores = HashMap::new();
    scores.insert(refs[3].clone(), 9.0);
    let order = scored_order(&refs, &scores);
    assert_eq!(
        order.order,
        vec![4, 1, 2, 3],
        "a partially scored arm degrades toward the baseline, not toward a shuffle"
    );
    assert_eq!(order.scored_fraction, 0.25);
}

#[test]
fn an_arm_with_no_scores_for_a_task_reproduces_the_baseline_order() {
    let refs = refs();
    let empty: HashMap<String, f64> = HashMap::new();
    assert_eq!(scored_order(&refs, &empty).order, baseline_order(4).order);
    assert_eq!(scored_order(&refs, &empty).scored_fraction, 0.0);
    assert_eq!(scored_order(&[], &empty).scored_fraction, 0.0);
}

#[test]
fn a_scores_file_naming_an_unknown_task_is_refused() {
    let (path, _tmp) = write_scores(r#"{"arm":"x","scores":{"nope":{}}}"#);
    let err = ArmScores::load(&path, &["real".to_string()]).expect_err("must refuse");
    assert!(err.to_string().contains("`nope`"), "{err}");
    assert!(
        err.to_string().contains("not in this benchmark set"),
        "{err}"
    );
}

#[test]
fn a_scores_file_may_not_claim_the_baseline_name() {
    let (path, _tmp) = write_scores(&format!(r#"{{"arm":"{BASELINE_ARM}","scores":{{}}}}"#));
    let err = ArmScores::load(&path, &[]).expect_err("must refuse");
    assert!(
        err.to_string().contains("reserved for the baseline"),
        "{err}"
    );
}

#[test]
fn a_score_json_cannot_represent_is_refused_naming_the_file() {
    // `1e400` overflows an f64, and JSON has no NaN or infinity literal, so the
    // parser is what refuses a non-finite score — before any arm is built.
    let (path, _tmp) = write_scores(r#"{"arm":"x","scores":{"t1":{"memory:a:b":1e400}}}"#);
    let err = ArmScores::load(&path, &["t1".to_string()]).expect_err("must refuse");
    let msg = err.to_string();
    assert!(msg.contains("scores.json"), "must name the file: {msg}");
    assert!(msg.contains("out of range"), "{msg}");
}

#[test]
fn an_empty_arm_name_is_refused() {
    let (path, _tmp) = write_scores(r#"{"arm":"  ","scores":{}}"#);
    let err = ArmScores::load(&path, &[]).expect_err("must refuse");
    assert!(err.to_string().contains("must not be empty"), "{err}");
}

#[test]
fn an_unknown_key_in_a_scores_file_is_refused_rather_than_ignored() {
    let (path, _tmp) = write_scores(r#"{"arm":"x","scores":{},"typo":1}"#);
    let err = ArmScores::load(&path, &[]).expect_err("must refuse");
    assert!(err.to_string().contains("typo"), "{err}");
}

#[test]
fn a_missing_scores_file_names_the_path() {
    let err =
        ArmScores::load(&PathBuf::from("/nonexistent/scores.json"), &[]).expect_err("must refuse");
    assert!(
        err.to_string().contains("/nonexistent/scores.json"),
        "{err}"
    );
}

#[test]
fn a_valid_scores_file_carries_its_scorer_version_through() {
    let (path, _tmp) = write_scores(
        r#"{"arm":"base-ce","scorer_version":"bge@1","scores":{"t1":{"memory:a:b":2.5}}}"#,
    );
    let scores = ArmScores::load(&path, &["t1".to_string()]).expect("load");
    assert_eq!(scores.arm, "base-ce");
    assert_eq!(scores.scorer_version.as_deref(), Some("bge@1"));
    assert_eq!(
        scores.for_task("t1").and_then(|m| m.get("memory:a:b")),
        Some(&2.5)
    );
    assert!(scores.for_task("t2").is_none());
}
