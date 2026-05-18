use qwick::config::file::{AutoReindexMode, Config};

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
    std::env::set_var("QWICK_INDEXING_AUTO_REINDEX", "hook");
    std::env::set_var("QWICK_RETRIEVAL_TOP_K", "20");
    let c = Config::defaults().with_env();
    std::env::remove_var("QWICK_INDEXING_AUTO_REINDEX");
    std::env::remove_var("QWICK_RETRIEVAL_TOP_K");
    assert!(matches!(c.indexing.auto_reindex, AutoReindexMode::Hook));
    assert_eq!(c.retrieval.top_k, 20);
}

#[test]
fn toml_round_trip() {
    let c = Config::defaults();
    let s = toml::to_string(&c).expect("serialize defaults to toml");
    let back: Config = toml::from_str(&s).expect("deserialize defaults from toml");
    assert_eq!(back.retrieval.top_k, c.retrieval.top_k);
}
