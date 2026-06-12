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
