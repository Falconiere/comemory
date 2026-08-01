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
fn document_leg_weight_rejects_non_finite_and_out_of_range() {
    for bad in ["nan", "inf", "0.0", "-1.0", "10.1"] {
        assert_rejected(
            &format!("[retrieval]\ndocument_leg_weight = {bad}\n"),
            "retrieval.document_leg_weight",
        );
    }
    for ok in ["0.1", "0.5", "10.0"] {
        load(&format!("[retrieval]\ndocument_leg_weight = {ok}\n"))
            .expect("boundary document_leg_weight must be accepted");
    }
}

#[test]
fn document_leg_weight_env_var_is_named_in_the_error() {
    let err = load("[retrieval]\ndocument_leg_weight = 0.0\n")
        .expect_err("document_leg_weight=0.0 must be rejected");
    assert!(
        err.to_string()
            .contains("COMEMORY_RETRIEVAL_DOCUMENT_LEG_WEIGHT"),
        "error must name the env var, got: {err}"
    );
}

#[test]
fn max_file_bytes_rejects_zero() {
    assert_rejected(
        "[indexing]\nmax_file_bytes = 0\n",
        "indexing.max_file_bytes",
    );
    let cfg = load("[indexing]\nmax_file_bytes = 1\n").expect("1 byte must be accepted");
    assert_eq!(cfg.indexing.max_file_bytes, 1);
}

#[test]
fn max_file_bytes_env_var_is_named_in_the_error() {
    let err =
        load("[indexing]\nmax_file_bytes = 0\n").expect_err("max_file_bytes=0 must be rejected");
    assert!(
        err.to_string().contains("COMEMORY_INDEXING_MAX_FILE_BYTES"),
        "error must name the env var, got: {err}"
    );
}

#[test]
fn indexing_overlay_applies_only_present_keys_and_rejects_unknown() {
    let cfg = load("[indexing]\nmax_file_bytes = 2048\n").expect("valid [indexing] key");
    assert_eq!(cfg.indexing.max_file_bytes, 2048);
    // Absent keys keep their defaults.
    assert_eq!(cfg.indexing.auto_reindex_threshold_ms, 200);
    assert_eq!(cfg.indexing.incremental_batch_size, 50);

    assert_rejected("[indexing]\nnonexistent_key = 1\n", "nonexistent_key");
}

#[test]
fn graph_hops_rejects_over_four() {
    // The edge walk is bounded: past 4 hops a recursive CTE over a dense
    // `edges` table stops being a cheap expansion.
    assert_rejected("[retrieval]\ngraph_hops = 5\n", "retrieval.graph_hops");
    assert_rejected("[retrieval]\ngraph_hops = 9\n", "retrieval.graph_hops");
    // 0 (leg disabled) and 4 (the ceiling) are the inclusive bounds.
    for ok in ["0", "4"] {
        let cfg = load(&format!("[retrieval]\ngraph_hops = {ok}\n"))
            .expect("boundary graph_hops must be accepted");
        assert_eq!(cfg.retrieval.graph_hops.to_string(), ok);
    }
}

#[test]
fn graph_seeds_rejects_zero() {
    // Zero seeds would leave the walk nothing to start from — an
    // accidentally-silent way to disable the leg. `graph_hops = 0` is the
    // documented off switch.
    assert_rejected("[retrieval]\ngraph_seeds = 0\n", "retrieval.graph_seeds");
    let cfg = load("[retrieval]\ngraph_seeds = 1\n").expect("1 seed must be accepted");
    assert_eq!(cfg.retrieval.graph_seeds, 1);
}

#[test]
fn graph_knob_errors_name_the_env_var() {
    // Both entry points share one validate() pass, so the file-overlay
    // message must still point at the env var an operator may have set.
    for (body, var) in [
        (
            "[retrieval]\ngraph_hops = 9\n",
            "COMEMORY_RETRIEVAL_GRAPH_HOPS",
        ),
        (
            "[retrieval]\ngraph_seeds = 0\n",
            "COMEMORY_RETRIEVAL_GRAPH_SEEDS",
        ),
    ] {
        let err = load(body).expect_err("invalid graph knob must be rejected");
        let msg = err.to_string();
        assert!(msg.contains(var), "error must name '{var}', got: {msg}");
    }
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
         bm25_grid = [[1.0, 3.0]]\n\
         graph_hops_grid = [1]\n\
         graph_seeds_grid = [8]\n",
    )
    .expect("singleton grids must be accepted");
    assert_eq!(cfg.tune.rrf_k_grid, vec![60.0f32]);
    assert_eq!(cfg.tune.bm25_grid, vec![(1.0f32, 3.0f32)]);
    assert_eq!(cfg.tune.graph_hops_grid, vec![1u32]);
    assert_eq!(cfg.tune.graph_seeds_grid, vec![8usize]);
}

#[test]
fn empty_graph_grids_are_rejected() {
    // Same rule as every other pool: an empty list gives the sampler
    // nothing to draw and the grid no points.
    assert_rejected("[tune]\ngraph_hops_grid = []\n", "tune.graph_hops_grid");
    assert_rejected("[tune]\ngraph_seeds_grid = []\n", "tune.graph_seeds_grid");
}

#[test]
fn graph_grid_values_run_the_scalar_checks() {
    // Each pool value passes the bounds its scalar knob enforces — hops
    // beyond the walk ceiling, or a zero seed count, fail exactly like
    // `retrieval.graph_hops = 9` / `retrieval.graph_seeds = 0`.
    assert_rejected("[tune]\ngraph_hops_grid = [0, 9]\n", "tune.graph_hops_grid");
    assert_rejected(
        "[tune]\ngraph_seeds_grid = [8, 0]\n",
        "tune.graph_seeds_grid",
    );
}

#[test]
fn tune_samples_takes_any_usize() {
    // `samples` is a budget, not a bounded knob: 0 selects the exhaustive
    // cartesian grid and an oversized value is clamped by the sampler.
    for n in ["0", "1", "100000"] {
        let cfg = load(&format!("[tune]\nsamples = {n}\n"))
            .expect("any usize samples value must be accepted");
        assert_eq!(cfg.tune.samples.to_string(), n);
    }
}
