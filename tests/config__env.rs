//! Mirrors `src/config/env.rs` — `COMEMORY_*` env-var override behavior.
//!
//! Every test here mutates process-global env vars. Under nextest each
//! test runs in its own process; under plain `cargo test` this binary
//! must run with `--test-threads=1` (see `.config/nextest.toml`).

use comemory::config::file::{AutoReindexMode, Config};

#[test]
fn env_overrides_apply() {
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_INDEXING_AUTO_REINDEX", "hook") };
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RETRIEVAL_TOP_K", "20") };
    let c = Config::defaults().with_env().unwrap();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_INDEXING_AUTO_REINDEX") };
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_TOP_K") };
    assert!(matches!(c.indexing.auto_reindex, AutoReindexMode::Hook));
    assert_eq!(c.retrieval.top_k, 20);
}

#[test]
fn env_rrf_k_override_applies() {
    // Regression for C3: the CLI must read rrf_k through Config instead of
    // hardcoding 60.0. Verify the env var path drops a valid override into
    // the retrieval config so callers (search, context) can consume it.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RETRIEVAL_RRF_K", "42.0") };
    let c = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_RRF_K") };
    let cfg = c.expect("rrf_k override must succeed");
    assert!((cfg.retrieval.rrf_k - 42.0).abs() < 1e-6);
}

#[test]
fn env_invalid_rrf_k_returns_err() {
    // Regression for G3: NaN / inf / non-positive rrf_k values must surface
    // as `Err` rather than silently falling back to the default. They would
    // cause `1 / (k + rank)` to collapse to NaN or pin every score to the
    // same bucket, silently destroying the ranking — so a typo must abort
    // at startup, matching the style used by `top_k` / `memory_threshold`.
    for bad in ["nan", "NaN", "inf", "-inf", "0", "-1"] {
        // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
        unsafe { std::env::set_var("COMEMORY_RETRIEVAL_RRF_K", bad) };
        let result = Config::defaults().with_env();
        // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
        unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_RRF_K") };
        let err = result.expect_err(&format!("'{bad}' must error"));
        let msg = err.to_string();
        assert!(
            msg.contains("COMEMORY_RETRIEVAL_RRF_K"),
            "error must name the offending var for '{bad}', got: {msg}"
        );
    }
}

#[test]
fn env_invalid_memory_threshold_returns_err() {
    // Out-of-range or non-finite memory_threshold must abort at startup
    // exactly like code_threshold does — before the validate() arm landed,
    // COMEMORY_RETRIEVAL_MEMORY_THRESHOLD=5 passed silently and the ANN
    // floor dropped every hit.
    for bad in ["5", "1.5", "-0.1", "nan", "inf"] {
        // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
        unsafe { std::env::set_var("COMEMORY_RETRIEVAL_MEMORY_THRESHOLD", bad) };
        let result = Config::defaults().with_env();
        // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
        unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_MEMORY_THRESHOLD") };
        let err = result.expect_err(&format!("'{bad}' must error"));
        let msg = err.to_string();
        assert!(
            msg.contains("COMEMORY_RETRIEVAL_MEMORY_THRESHOLD"),
            "error must name the offending var for '{bad}', got: {msg}"
        );
    }
    // Boundary values pass: 0.0 disables the floor, 1.0 demands
    // exact-match similarity.
    for ok in ["0.0", "1.0"] {
        // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
        unsafe { std::env::set_var("COMEMORY_RETRIEVAL_MEMORY_THRESHOLD", ok) };
        let result = Config::defaults().with_env();
        // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
        unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_MEMORY_THRESHOLD") };
        result.expect("boundary memory_threshold must be accepted");
    }
}

#[test]
fn env_bm25_weights_override_applies() {
    // COMEMORY_RETRIEVAL_BM25_WEIGHTS sets the (body, tags) weighted-BM25
    // pair consumed by the memory FTS search.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RETRIEVAL_BM25_WEIGHTS", "2.0,1.0") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_BM25_WEIGHTS") };
    let cfg = result.expect("valid bm25_weights override must succeed");
    assert_eq!(cfg.retrieval.bm25_weights, (2.0, 1.0));
}

#[test]
fn env_bm25_weights_invalid_is_an_error() {
    // A non-numeric component must surface as Err naming the variable, not
    // silently fall back to the default weights.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RETRIEVAL_BM25_WEIGHTS", "x,1") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_BM25_WEIGHTS") };
    let err = result.expect_err("non-numeric bm25 weight must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_RETRIEVAL_BM25_WEIGHTS"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn env_bm25_weights_zero_pair_is_an_error() {
    // Both-zero weights would zero out every BM25 score; validate() must
    // reject the pair just like the file overlay does.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RETRIEVAL_BM25_WEIGHTS", "0,0") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_BM25_WEIGHTS") };
    let err = result.expect_err("zero bm25 weight pair must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_RETRIEVAL_BM25_WEIGHTS"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn env_rejects_invalid_top_k() {
    // Non-numeric top_k must surface as Err instead of silently keeping the
    // default; otherwise typos go unnoticed until retrieval misbehaves.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RETRIEVAL_TOP_K", "not-a-number") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RETRIEVAL_TOP_K") };
    let err = result.expect_err("invalid top_k must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_RETRIEVAL_TOP_K"),
        "error message should name the offending var, got: {msg}"
    );
}

#[test]
fn env_rejects_invalid_auto_reindex() {
    // Unknown mode (typo of "hook") must error, not fall through to Lazy.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_INDEXING_AUTO_REINDEX", "hooks") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_INDEXING_AUTO_REINDEX") };
    let err = result.expect_err("unknown auto_reindex must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_INDEXING_AUTO_REINDEX"),
        "error message should name the offending var, got: {msg}"
    );
}

#[test]
fn env_rejects_invalid_auto_sync() {
    // Boolean parser only accepts true|1|yes|on / false|0|no|off; anything
    // else (e.g. "maybe") must error.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_GIT_AUTO_SYNC", "maybe") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_GIT_AUTO_SYNC") };
    let err = result.expect_err("unknown auto_sync must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_GIT_AUTO_SYNC"),
        "error message should name the offending var, got: {msg}"
    );
}

#[test]
fn env_vector_dim_is_ignored() {
    // Regression for PR #3 review thread: COMEMORY_VECTOR_DIM used to feed
    // `with_env` and silently overwrite `retrieval.memory_vector_dim`,
    // creating a footgun where the config value disagreed with the
    // hardcoded vec0 DDL. `with_env` must leave the field on its default
    // even when the env var is set.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_VECTOR_DIM", "512") };
    let c = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_VECTOR_DIM") };
    let cfg = c.expect("with_env must succeed even with COMEMORY_VECTOR_DIM set");
    assert_eq!(
        cfg.retrieval.memory_vector_dim, 1024,
        "COMEMORY_VECTOR_DIM must not override the DDL-locked dim"
    );
}

#[test]
fn env_code_vector_dim_is_ignored() {
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_CODE_VECTOR_DIM", "384") };
    let c = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_CODE_VECTOR_DIM") };
    let cfg = c.expect("with_env must succeed even with COMEMORY_CODE_VECTOR_DIM set");
    assert_eq!(
        cfg.retrieval.code_vector_dim, 768,
        "COMEMORY_CODE_VECTOR_DIM must not override the DDL-locked dim"
    );
}

#[test]
fn env_embed_hint_overrides_apply() {
    // Doctor surfaces this verbatim so callers know which embedder filled
    // the vectors. It is opaque to comemory itself.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_EMBED_HINT", "ollama:nomic-embed-text") };
    let c = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_EMBED_HINT") };
    let cfg = c.expect("embed hint override must succeed");
    assert_eq!(cfg.embed_hint.as_deref(), Some("ollama:nomic-embed-text"));
}

// ── Rank + prune env knobs (M1) ──────────────────────────────────────────────

#[test]
fn rank_env_overrides() {
    // All five new rank/prune env vars must be picked up by with_env().
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RANK_DECAY", "0.7") };
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RANK_PRIOR_CLAMP", "0.6,1.8") };
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RANK_MMR_LAMBDA", "0.5") };
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_PRUNE_MIN_ACTIVATION", "-1.5") };
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_PRUNE_MIN_FEEDBACK", "0.3") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RANK_DECAY") };
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RANK_PRIOR_CLAMP") };
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RANK_MMR_LAMBDA") };
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_PRUNE_MIN_ACTIVATION") };
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_PRUNE_MIN_FEEDBACK") };
    let cfg = result.expect("all valid rank/prune env vars must succeed");
    assert!((cfg.rank.decay - 0.7).abs() < f64::EPSILON);
    assert_eq!(cfg.rank.prior_clamp, (0.6, 1.8));
    assert!((cfg.rank.mmr_lambda - 0.5).abs() < f64::EPSILON);
    assert!((cfg.prune.min_activation - (-1.5)).abs() < f64::EPSILON);
    assert!((cfg.prune.min_feedback - 0.3).abs() < f64::EPSILON);
}

#[test]
fn bad_clamp_lo_gt_hi_is_an_error() {
    // lo > hi is an invalid configuration: the rerank prior clamp requires
    // a valid [lo, hi] interval. This must surface as Err, not silent no-op.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RANK_PRIOR_CLAMP", "2.0,0.5") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RANK_PRIOR_CLAMP") };
    let err = result.expect_err("lo > hi prior_clamp must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_RANK_PRIOR_CLAMP"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn bad_decay_negative_is_an_error() {
    // Negative decay violates the ACT-R formula (ln(n) - d*ln(days+1)):
    // a negative d would invert recency weighting. Must surface as Err.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RANK_DECAY", "-1.0") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RANK_DECAY") };
    let err = result.expect_err("negative decay must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_RANK_DECAY"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn bad_mmr_lambda_out_of_range_is_an_error() {
    // MMR lambda must be in [0, 1]. A value > 1 has no defined meaning in
    // the relevance-vs-diversity trade-off and must be rejected.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_RANK_MMR_LAMBDA", "2.0") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_RANK_MMR_LAMBDA") };
    let err = result.expect_err("lambda > 1 must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_RANK_MMR_LAMBDA"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn bad_min_feedback_out_of_range_is_an_error() {
    // min_feedback is a beta-feedback value in [0, 1]. A value > 1 means no
    // memory is ever prune-eligible, which is a configuration error.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_PRUNE_MIN_FEEDBACK", "1.5") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_PRUNE_MIN_FEEDBACK") };
    let err = result.expect_err("min_feedback > 1 must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_PRUNE_MIN_FEEDBACK"),
        "error must name the offending var, got: {msg}"
    );
}

#[test]
fn env_prune_below_quality_override_applies() {
    // COMEMORY_PRUNE_BELOW_QUALITY newly wired to the existing field
    // low_value_default_below_quality. Valid range 1..=5.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_PRUNE_BELOW_QUALITY", "3") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_PRUNE_BELOW_QUALITY") };
    let cfg = result.expect("valid below_quality override must succeed");
    assert_eq!(cfg.prune.low_value_default_below_quality, 3);
}

#[test]
fn bad_prune_below_quality_out_of_range_is_an_error() {
    // Quality 0 is below the minimum allowed value of 1.
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_PRUNE_BELOW_QUALITY", "0") };
    let result = Config::defaults().with_env();
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_PRUNE_BELOW_QUALITY") };
    let err = result.expect_err("below_quality=0 must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_PRUNE_BELOW_QUALITY"),
        "error must name the offending var, got: {msg}"
    );
}
