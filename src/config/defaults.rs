//! Default-value functions for [`super::file::Config`]'s `#[serde(default
//! = "...")]` attributes.
//!
//! Split out of `file.rs` to keep that module under the 300-code-line cap:
//! `file.rs` owns the struct definitions, the file overlay, and
//! `Config::defaults`; this module owns the tiny per-field default
//! constructors those two consumers share. Each fn is `pub(crate)` so the
//! serde attribute paths in `file.rs` and the `Config::defaults` body can
//! both reach them.

/// Default memory embedding dim (reporting-only; the authoritative value is
/// the `memory_vec` DDL literal).
pub(crate) fn default_memory_vector_dim() -> usize {
    1024
}

/// Default ceiling on how deep a paginated retrieval can page into the
/// ranked list — see [`super::file::RetrievalConfig::max_page_window`].
pub(crate) fn default_max_page_window() -> usize {
    200
}

/// Default `(body, tags)` BM25 column weights for `memory_fts`.
pub(crate) fn default_bm25_weights() -> (f32, f32) {
    (1.0, 3.0)
}

/// Default minimum cosine similarity for code ANN hits.
pub(crate) fn default_code_threshold() -> f32 {
    0.50
}

/// Default `(symbol, snippet, path_tokens)` BM25 column weights for
/// `code_fts`.
pub(crate) fn default_code_bm25_weights() -> (f32, f32, f32) {
    (2.0, 1.0, 1.5)
}

/// Default code embedding dim (reporting-only; the authoritative value is
/// the `code_vec` DDL literal).
pub(crate) fn default_code_vector_dim() -> usize {
    768
}

/// The shared constant in `simhash` stays the single source of the default
/// radius; the config field merely makes it operator-tunable.
pub(crate) fn default_near_dup_hamming() -> u32 {
    crate::simhash::NEAR_DUP_HAMMING
}

/// The constant next to the prune rule stays the single source of the
/// default grace window; the config field merely makes it operator-tunable.
pub(crate) fn default_superseded_grace_days() -> u32 {
    crate::prune::low_value::SUPERSEDED_GRACE_DAYS
}
