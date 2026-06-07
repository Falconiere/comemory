use comemory::config::file::{AutoReindexMode, Config};

#[test]
fn defaults_match_spec() {
    let c = Config::defaults();
    assert_eq!(c.embeddings.memory_model, "nomic-embed-text-v1.5-Q");
    assert_eq!(c.embeddings.code_model, "jina-embeddings-v2-base-code-Q");
    assert!(matches!(c.indexing.auto_reindex, AutoReindexMode::Lazy));
    assert_eq!(c.indexing.auto_reindex_threshold_ms, 200);
    assert_eq!(c.retrieval.memory_threshold, 0.55);
    assert_eq!(c.retrieval.code_threshold, 0.50);
    assert_eq!(c.retrieval.hybrid_weight, 0.65);
    assert_eq!(c.retrieval.top_k, 12);
    assert_eq!(c.prune.trash_retention_days, 30);
    assert!(
        (c.retrieval.rrf_k - 60.0).abs() < f32::EPSILON,
        "default rrf_k must be 60.0"
    );
}

#[test]
fn env_overrides_apply() {
    // SAFETY: serial within this test; env vars scoped and removed after read.
    std::env::set_var("COMEMORY_INDEXING_AUTO_REINDEX", "hook");
    std::env::set_var("COMEMORY_RETRIEVAL_TOP_K", "20");
    let c = Config::defaults().with_env().unwrap();
    std::env::remove_var("COMEMORY_INDEXING_AUTO_REINDEX");
    std::env::remove_var("COMEMORY_RETRIEVAL_TOP_K");
    assert!(matches!(c.indexing.auto_reindex, AutoReindexMode::Hook));
    assert_eq!(c.retrieval.top_k, 20);
}

#[test]
fn env_rrf_k_override_applies() {
    // Regression for C3: the CLI must read rrf_k through Config instead of
    // hardcoding 60.0. Verify the env var path drops a valid override into
    // the retrieval config so callers (search, context) can consume it.
    //
    // This test exercises only one env var and reads through `with_env()`
    // immediately, mirroring the pre-existing env-var tests in this file.
    // Run with `--test-threads=1` to avoid races with other env-mutating
    // tests (a pre-existing constraint of this test binary).
    std::env::set_var("COMEMORY_RETRIEVAL_RRF_K", "42.0");
    let c = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_RETRIEVAL_RRF_K");
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
    //
    // Run with `--test-threads=1` (already required by this binary) so env
    // vars don't race with the other Config tests.
    for bad in ["nan", "NaN", "inf", "-inf", "0", "-1"] {
        std::env::set_var("COMEMORY_RETRIEVAL_RRF_K", bad);
        let result = Config::defaults().with_env();
        std::env::remove_var("COMEMORY_RETRIEVAL_RRF_K");
        let err = result.expect_err(&format!("'{bad}' must error"));
        let msg = err.to_string();
        assert!(
            msg.contains("COMEMORY_RETRIEVAL_RRF_K"),
            "error must name the offending var for '{bad}', got: {msg}"
        );
    }
}

#[test]
fn env_rejects_invalid_top_k() {
    // Non-numeric top_k must surface as Err instead of silently keeping the
    // default; otherwise typos go unnoticed until retrieval misbehaves.
    std::env::set_var("COMEMORY_RETRIEVAL_TOP_K", "not-a-number");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_RETRIEVAL_TOP_K");
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
    std::env::set_var("COMEMORY_INDEXING_AUTO_REINDEX", "hooks");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_INDEXING_AUTO_REINDEX");
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
    std::env::set_var("COMEMORY_GIT_AUTO_SYNC", "maybe");
    let result = Config::defaults().with_env();
    std::env::remove_var("COMEMORY_GIT_AUTO_SYNC");
    let err = result.expect_err("unknown auto_sync must error");
    let msg = err.to_string();
    assert!(
        msg.contains("COMEMORY_GIT_AUTO_SYNC"),
        "error message should name the offending var, got: {msg}"
    );
}

#[test]
fn toml_round_trip() {
    let c = Config::defaults();
    let s = toml::to_string(&c).expect("serialize defaults to toml");
    let back: Config = toml::from_str(&s).expect("deserialize defaults from toml");
    assert_eq!(back.retrieval.top_k, c.retrieval.top_k);
}
