#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirrors `src/config/retrieval.rs` — `RetrievalConfig` defaults, the
//! `[retrieval]` file overlay, and the import paths the type is reachable
//! from after it moved out of `config::file`.

use comemory::config::Config;

#[test]
fn retrieval_defaults_match_spec() {
    let r = Config::defaults().retrieval;
    assert_eq!(r.memory_threshold, 0.55);
    assert_eq!(r.code_threshold, 0.50);
    assert_eq!(r.hybrid_weight, 0.65);
    assert_eq!(r.top_k, 12);
    assert_eq!(r.max_page_window, 200);
    assert_eq!(r.corrective_min_confidence, 0.15);
    assert_eq!(r.rrf_k, 60.0);
    assert_eq!(r.graph_hops, 2);
    assert_eq!(r.graph_seeds, 8);
    assert_eq!(r.bm25_weights, (1.0, 3.0));
    assert_eq!(r.code_bm25_weights, (2.0, 1.0, 1.5));
    assert_eq!(r.memory_vector_dim, 1024);
    assert_eq!(r.code_vector_dim, 768);
    assert_eq!(r.document_leg_weight, 0.5);
}

#[test]
fn retrieval_type_is_reachable_from_both_paths() {
    // The struct lives in `config::retrieval` now; `config::file` keeps a
    // re-export so historical `use comemory::config::file::RetrievalConfig`
    // imports still compile.
    let _via_module: comemory::config::retrieval::RetrievalConfig = Config::defaults().retrieval;
    let _via_file: comemory::config::file::RetrievalConfig = Config::defaults().retrieval;
    let _via_root: comemory::config::RetrievalConfig = Config::defaults().retrieval;
}

#[test]
fn retrieval_overlay_applies_only_present_keys() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[retrieval]\n\
         rrf_k = 20.0\n\
         top_k = 5\n\
         max_page_window = 50\n\
         memory_threshold = 0.7\n\
         code_threshold = 0.9\n\
         code_bm25_weights = [3.0, 2.0, 1.0]\n",
    )
    .expect("write config.toml");
    let r = Config::defaults()
        .with_file(&path)
        .expect("valid [retrieval] keys must parse and apply")
        .retrieval;
    assert_eq!(r.rrf_k, 20.0);
    assert_eq!(r.top_k, 5);
    assert_eq!(r.max_page_window, 50);
    assert_eq!(r.memory_threshold, 0.7);
    assert_eq!(r.code_threshold, 0.9);
    assert_eq!(r.code_bm25_weights, (3.0, 2.0, 1.0));
    // Keys absent from the file keep their defaults.
    assert_eq!(r.bm25_weights, (1.0, 3.0));
    assert_eq!(r.graph_hops, 2);
    assert_eq!(r.graph_seeds, 8);
    assert_eq!(r.hybrid_weight, 0.65);
    assert_eq!(r.memory_vector_dim, 1024);
    // document_leg_weight is absent from this file, so it keeps its default.
    assert_eq!(r.document_leg_weight, 0.5);
}

#[test]
fn retrieval_overlay_applies_document_leg_weight() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[retrieval]\ndocument_leg_weight = 0.9\n").expect("write config.toml");
    let r = Config::defaults()
        .with_file(&path)
        .expect("valid document_leg_weight must parse and apply")
        .retrieval;
    assert_eq!(r.document_leg_weight, 0.9);
}

#[test]
fn retrieval_overlay_rejects_non_overlayable_keys() {
    // Structural knobs (the vector dims) and the two unused blend weights
    // are deliberately absent from the partial: `deny_unknown_fields` turns
    // an attempt to set them into a loud startup error rather than a
    // silently-ignored line in config.toml.
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    for key in ["memory_vector_dim = 512", "hybrid_weight = 0.9"] {
        std::fs::write(&path, format!("[retrieval]\n{key}\n")).expect("write config.toml");
        let err = Config::defaults()
            .with_file(&path)
            .expect_err("non-overlayable [retrieval] key must be rejected");
        let name = key.split(' ').next().unwrap_or(key);
        assert!(
            err.to_string().contains(name),
            "error must name the rejected key '{name}', got: {err}"
        );
    }
}

#[test]
fn retrieval_overlay_runs_validation() {
    // The overlay funnels through the same `Config::validate` pass as the
    // env layer, so an out-of-range value from config.toml hard-errors.
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[retrieval]\nmax_page_window = 0\n").expect("write config.toml");
    let err = Config::defaults()
        .with_file(&path)
        .expect_err("max_page_window=0 must error");
    assert!(
        err.to_string().contains("retrieval.max_page_window"),
        "error must name the offending field, got: {err}"
    );
}
