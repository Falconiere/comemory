//! Mirrors `src/config/validate.rs` — the shared invariant pass both the
//! file overlay and the env layer funnel through. Exercised via the public
//! `Config::with_file` entry point so the tests cover exactly what an
//! operator-authored `config.toml` hits.

use comemory::config::Config;
use comemory::errors::Error;

/// Write `body` to a temp config.toml and run the file overlay (which ends
/// in `Config::validate`).
fn load(body: &str) -> std::result::Result<Config, Error> {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, body).expect("write config.toml");
    Config::defaults().with_file(&path)
}

/// Assert that `body` is rejected with a message naming `field`.
fn assert_rejected(body: &str, field: &str) {
    let err = load(body).expect_err(&format!("'{body}' must be rejected"));
    let msg = err.to_string();
    assert!(
        msg.contains(field),
        "error must name '{field}' for '{body}', got: {msg}"
    );
}

#[test]
fn memory_threshold_rejects_non_finite_and_out_of_range() {
    // Symmetry with code_threshold: before this arm existed,
    // `memory_threshold = 5` passed silently and the ANN floor dropped
    // every hit.
    for bad in ["nan", "inf", "5", "1.5", "-0.1"] {
        assert_rejected(
            &format!("[retrieval]\nmemory_threshold = {bad}\n"),
            "retrieval.memory_threshold",
        );
    }
    // Boundary values are valid: 0.0 disables the floor, 1.0 demands
    // exact-match similarity.
    for ok in ["0.0", "1.0"] {
        load(&format!("[retrieval]\nmemory_threshold = {ok}\n"))
            .expect("boundary memory_threshold must be accepted");
    }
}

#[test]
fn code_threshold_rejects_non_finite_and_out_of_range() {
    for bad in ["nan", "inf", "1.5", "-0.1"] {
        assert_rejected(
            &format!("[retrieval]\ncode_threshold = {bad}\n"),
            "retrieval.code_threshold",
        );
    }
    // Boundary values are valid: 0.0 disables the floor, 1.0 demands
    // exact-match similarity.
    for ok in ["0.0", "1.0"] {
        load(&format!("[retrieval]\ncode_threshold = {ok}\n"))
            .expect("boundary code_threshold must be accepted");
    }
}

#[test]
fn code_bm25_weights_rejects_negative_nonfinite_and_all_zero() {
    for bad in [
        "[-1.0, 1.0, 1.0]",
        "[1.0, nan, 1.0]",
        "[1.0, 1.0, inf]",
        "[0.0, 0.0, 0.0]",
    ] {
        assert_rejected(
            &format!("[retrieval]\ncode_bm25_weights = {bad}\n"),
            "retrieval.code_bm25_weights",
        );
    }
    // A single positive column is enough.
    load("[retrieval]\ncode_bm25_weights = [0.0, 0.0, 1.0]\n")
        .expect("one positive weight must be accepted");
}

#[test]
fn near_dup_hamming_rejects_over_64() {
    assert_rejected("[rank]\nnear_dup_hamming = 65\n", "rank.near_dup_hamming");
    // 64 (the whole hash) and 0 (collapse only identical hashes) are the
    // inclusive bounds.
    let cfg = load("[rank]\nnear_dup_hamming = 64\n").expect("64 must be accepted");
    assert_eq!(cfg.rank.near_dup_hamming, 64);
    let cfg = load("[rank]\nnear_dup_hamming = 0\n").expect("0 must be accepted");
    assert_eq!(cfg.rank.near_dup_hamming, 0);
}

#[test]
fn empty_tune_grids_are_rejected() {
    // An empty grid would make `comemory tune` evaluate nothing and crown
    // no winner; each list must carry at least one point.
    assert_rejected("[tune]\nrrf_k_grid = []\n", "tune.rrf_k_grid");
    assert_rejected("[tune]\ndecay_grid = []\n", "tune.decay_grid");
    assert_rejected("[tune]\nmmr_lambda_grid = []\n", "tune.mmr_lambda_grid");
    assert_rejected("[tune]\nbm25_grid = []\n", "tune.bm25_grid");
}

#[test]
fn tune_grid_values_run_the_scalar_checks() {
    // Each grid value passes through the same bounds its scalar knob
    // enforces — a grid containing rrf_k 0.0 fails exactly like
    // `retrieval.rrf_k = 0.0` would.
    assert_rejected("[tune]\nrrf_k_grid = [60.0, 0.0]\n", "tune.rrf_k_grid");
    assert_rejected("[tune]\nrrf_k_grid = [nan]\n", "tune.rrf_k_grid");
    assert_rejected("[tune]\ndecay_grid = [0.5, -1.0]\n", "tune.decay_grid");
    assert_rejected(
        "[tune]\nmmr_lambda_grid = [0.7, 2.0]\n",
        "tune.mmr_lambda_grid",
    );
    assert_rejected("[tune]\nbm25_grid = [[0.0, 0.0]]\n", "tune.bm25_grid");
    assert_rejected("[tune]\nbm25_grid = [[-1.0, 3.0]]\n", "tune.bm25_grid");
}

#[test]
fn valid_singleton_grids_are_accepted() {
    let cfg = load(
        "[tune]\n\
         rrf_k_grid = [60.0]\n\
         decay_grid = [0.5]\n\
         mmr_lambda_grid = [0.7]\n\
         bm25_grid = [[1.0, 3.0]]\n",
    )
    .expect("singleton grids must be accepted");
    assert_eq!(cfg.tune.rrf_k_grid, vec![60.0f32]);
    assert_eq!(cfg.tune.bm25_grid, vec![(1.0f32, 3.0f32)]);
}
