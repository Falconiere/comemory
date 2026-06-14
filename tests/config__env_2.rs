//! Mirrors `src/config/env.rs` — `COMEMORY_*` env-var override behavior (part 2).
//!
//! Every test here mutates process-global env vars. Under nextest each
//! test runs in its own process; under plain `cargo test` this binary
//! must run with `--test-threads=1` (see `.config/nextest.toml`).

use comemory::config::file::Config;

#[test]
fn env_learning_retention_days_override_applies() {
    // COMEMORY_LEARNING_RETENTION_DAYS feeds prune.learning_retention_days,
    // the telemetry retention window consumed by `comemory gc`.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_LEARNING_RETENTION_DAYS", "7") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_LEARNING_RETENTION_DAYS") };
    let cfg = result.expect("valid learning retention override must succeed");
    assert_eq!(cfg.prune.learning_retention_days, 7);
}

#[test]
fn env_learning_retention_days_zero_is_an_error() {
    // Retention must be >= 1 day; 0 would evict telemetry the moment it is
    // written. Same validate() pass as the file overlay.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_LEARNING_RETENTION_DAYS", "0") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_LEARNING_RETENTION_DAYS") };
    let err = result.expect_err("learning retention 0 must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_LEARNING_RETENTION_DAYS"),
        "error must name the offending var, got: {msg}"
    );
}

// ── M3 code knobs + configurable constants ───────────────────────────────────

#[test]
fn env_code_threshold_override_applies() {
    // Reintroduced in M3 alongside the code-search wiring that consumes it;
    // the env arm and validation are live from day one.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RETRIEVAL_CODE_THRESHOLD", "0.7") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_CODE_THRESHOLD") };
    let cfg = result.expect("valid code_threshold override must succeed");
    assert!((cfg.retrieval.code_threshold - 0.7).abs() < f32::EPSILON);
}

#[test]
fn env_code_threshold_invalid_is_an_error() {
    // Cosine similarity lives in [0, 1]; non-finite or out-of-range values
    // (and plain typos) must abort at startup naming the variable.
    for bad in ["not-a-number", "1.5", "-0.1", "nan"] {
        // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
        unsafe { std::env::set_var("COMEMORY_RETRIEVAL_CODE_THRESHOLD", bad) };
        let result = Config::defaults().with_env();
        // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
        unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_CODE_THRESHOLD") };
        let err = result.expect_err(&format!("'{bad}' must error"));
        let msg = err.to_string();
        assert!(
            msg.contains("COMEMORY_RETRIEVAL_CODE_THRESHOLD"),
            "error must name the offending var for '{bad}', got: {msg}"
        );
    }
}

#[test]
fn env_code_bm25_weights_triple_applies() {
    // "symbol,snippet,path" column order, mirroring the code_fts schema.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS", "3.0,1.0,2.0") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS") };
    let cfg = result.expect("valid code_bm25_weights override must succeed");
    assert_eq!(cfg.retrieval.code_bm25_weights, (3.0, 1.0, 2.0));
}

#[test]
fn env_code_bm25_weights_bad_shape_or_value_is_an_error() {
    // Wrong arity (pair instead of triple), non-numeric component, negative
    // weight, and the all-zero triple must all error naming the variable.
    for bad in ["2.0,1.0", "x,1,1", "-1,1,1", "0,0,0"] {
        // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
        unsafe { std::env::set_var("COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS", bad) };
        let result = Config::defaults().with_env();
        // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
        unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS") };
        let err = result.expect_err(&format!("'{bad}' must error"));
        let msg = err.to_string();
        assert!(
            msg.contains("COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS"),
            "error must name the offending var for '{bad}', got: {msg}"
        );
    }
}

#[test]
fn env_near_dup_hamming_override_applies() {
    // Tightens (or loosens) the SimHash near-duplicate radius consumed by
    // retrieval::diversify and the save-time duplicate warning.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RANK_NEAR_DUP_HAMMING", "4") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RANK_NEAR_DUP_HAMMING") };
    let cfg = result.expect("valid near_dup_hamming override must succeed");
    assert_eq!(cfg.rank.near_dup_hamming, 4);
}

#[test]
fn env_near_dup_hamming_over_64_is_an_error() {
    // SimHash is 64-bit, so a radius above 64 would collapse EVERY pair of
    // memories into one dup group. Must be rejected.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RANK_NEAR_DUP_HAMMING", "65") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RANK_NEAR_DUP_HAMMING") };
    let err = result.expect_err("near_dup_hamming > 64 must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_RANK_NEAR_DUP_HAMMING"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn env_superseded_grace_days_override_applies() {
    // Grace window for the superseded-and-forgotten prune rule.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS", "14") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS") };
    let cfg = result.expect("valid grace-days override must succeed");
    assert_eq!(cfg.prune.superseded_grace_days, 14);
}

#[test]
fn env_superseded_grace_days_invalid_is_an_error() {
    // u32 parse failure (negative or non-numeric) must error, not no-op.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS", "-1") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS") };
    let err = result.expect_err("negative grace days must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn tune_grids_have_no_env_overrides() {
    // Grids are file-only: a four-list env value is unreadable, so the env
    // layer deliberately offers no COMEMORY_TUNE_*_GRID vars. Setting one
    // must neither error nor change the grid.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_TUNE_RRF_K_GRID", "20.0,60.0") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_TUNE_RRF_K_GRID") };
    let cfg = result.expect("with_env must ignore unoffered tune grid vars");
    assert_eq!(cfg.tune.rrf_k_grid, vec![20.0f32, 60.0, 100.0]);
}

#[test]
fn env_prune_unused_since_days_is_ignored() {
    // The legacy low_value_default_unused_since_days knob was removed in
    // M2 (zero consumers since M1): setting the legacy var must neither
    // error nor change any prune knob.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_PRUNE_UNUSED_SINCE_DAYS", "90") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_PRUNE_UNUSED_SINCE_DAYS") };
    let cfg = result.expect("with_env must succeed with the legacy var set");
    assert_eq!(
        cfg.prune.trash_retention_days, 30,
        "legacy env var must not touch any prune field"
    );
}

#[test]
fn bad_clamp_lo_zero_is_an_error() {
    // lo must be > 0 per spec — a zero lower bound would collapse the clamp
    // interval to a single point and defeat multiplier bounding.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RANK_PRIOR_CLAMP", "0.0,1.5") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RANK_PRIOR_CLAMP") };
    let err = result.expect_err("clamp lo=0 must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_RANK_PRIOR_CLAMP"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn bad_clamp_wrong_arity_is_an_error() {
    // Only "lo,hi" (two values) is accepted. A bare scalar or triple is invalid.
    for bad in ["0.5", "0.5,1.0,1.5"] {
        // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
        unsafe { std::env::set_var("COMEMORY_RANK_PRIOR_CLAMP", bad) };
        let result = Config::defaults().with_env();
        // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
        unsafe { std::env::remove_var("COMEMORY_RANK_PRIOR_CLAMP") };
        let err = result.expect_err(&format!("bad clamp arity '{bad}' must error"));
        let msg = err.to_string();
        assert!(
            msg.contains("COMEMORY_RANK_PRIOR_CLAMP"),
            "error must name the offending var for '{bad}', got: {msg}"
        );
    }
}

#[test]
fn env_max_page_window_override_applies() {
    // COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW caps how deep pagination can reach.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW", "75") };
    let c = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW") };
    assert_eq!(
        c.expect("override must succeed").retrieval.max_page_window,
        75
    );
}

#[test]
fn env_zero_max_page_window_returns_err() {
    // 0 is invalid — the window must admit at least one page.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW", "0") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW") };
    let err = result.expect_err("0 max_page_window must error");
    assert!(
        err.to_string()
            .contains("COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW"),
        "error must name the offending var, got: {err}"
    );
}
