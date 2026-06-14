//! Mirror for `src/config/defaults.rs`. The default-value constructors are
//! `pub(crate)` (serde-attribute helpers), so they are pinned here through
//! the public `Config::defaults()` surface they feed — the same values must
//! land on the built config.

use comemory::config::file::Config;

#[test]
fn defaults_module_feeds_expected_retrieval_values() {
    let c = Config::defaults();
    assert_eq!(c.retrieval.max_page_window, 200, "default_max_page_window");
    assert_eq!(
        c.retrieval.memory_vector_dim, 1024,
        "default_memory_vector_dim"
    );
    assert_eq!(c.retrieval.code_vector_dim, 768, "default_code_vector_dim");
    assert_eq!(c.retrieval.code_threshold, 0.50, "default_code_threshold");
    assert_eq!(c.retrieval.bm25_weights, (1.0, 3.0), "default_bm25_weights");
    assert_eq!(
        c.retrieval.code_bm25_weights,
        (2.0, 1.0, 1.5),
        "default_code_bm25_weights"
    );
}

#[test]
fn defaults_module_feeds_expected_rank_and_prune_values() {
    let c = Config::defaults();
    assert_eq!(
        c.rank.near_dup_hamming,
        comemory::simhash::NEAR_DUP_HAMMING,
        "default_near_dup_hamming tracks the simhash constant"
    );
    // default_superseded_grace_days tracks the prune-rule constant (7).
    assert_eq!(
        c.prune.superseded_grace_days, 7,
        "default_superseded_grace_days"
    );
}
