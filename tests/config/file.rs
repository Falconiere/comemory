use qwick_memory::config::file::{AutoReindexMode, Config};

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
}

#[test]
fn env_overrides_apply() {
    // SAFETY: serial within this test; env vars scoped and removed after read.
    std::env::set_var("QWICK_MEMORY_INDEXING_AUTO_REINDEX", "hook");
    std::env::set_var("QWICK_MEMORY_RETRIEVAL_TOP_K", "20");
    let c = Config::defaults().with_env().unwrap();
    std::env::remove_var("QWICK_MEMORY_INDEXING_AUTO_REINDEX");
    std::env::remove_var("QWICK_MEMORY_RETRIEVAL_TOP_K");
    assert!(matches!(c.indexing.auto_reindex, AutoReindexMode::Hook));
    assert_eq!(c.retrieval.top_k, 20);
}

#[test]
fn env_rejects_invalid_top_k() {
    // Non-numeric top_k must surface as Err instead of silently keeping the
    // default; otherwise typos go unnoticed until retrieval misbehaves.
    std::env::set_var("QWICK_MEMORY_RETRIEVAL_TOP_K", "not-a-number");
    let result = Config::defaults().with_env();
    std::env::remove_var("QWICK_MEMORY_RETRIEVAL_TOP_K");
    let err = result.expect_err("invalid top_k must error");
    let msg = err.to_string();
    assert!(
        msg.contains("QWICK_MEMORY_RETRIEVAL_TOP_K"),
        "error message should name the offending var, got: {msg}"
    );
}

#[test]
fn env_rejects_invalid_auto_reindex() {
    // Unknown mode (typo of "hook") must error, not fall through to Lazy.
    std::env::set_var("QWICK_MEMORY_INDEXING_AUTO_REINDEX", "hooks");
    let result = Config::defaults().with_env();
    std::env::remove_var("QWICK_MEMORY_INDEXING_AUTO_REINDEX");
    let err = result.expect_err("unknown auto_reindex must error");
    let msg = err.to_string();
    assert!(
        msg.contains("QWICK_MEMORY_INDEXING_AUTO_REINDEX"),
        "error message should name the offending var, got: {msg}"
    );
}

#[test]
fn env_rejects_invalid_auto_sync() {
    // Boolean parser only accepts true|1|yes|on / false|0|no|off; anything
    // else (e.g. "maybe") must error.
    std::env::set_var("QWICK_MEMORY_GIT_AUTO_SYNC", "maybe");
    let result = Config::defaults().with_env();
    std::env::remove_var("QWICK_MEMORY_GIT_AUTO_SYNC");
    let err = result.expect_err("unknown auto_sync must error");
    let msg = err.to_string();
    assert!(
        msg.contains("QWICK_MEMORY_GIT_AUTO_SYNC"),
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
