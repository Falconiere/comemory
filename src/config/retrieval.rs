//! Retrieval knobs: the [`RetrievalConfig`] section and its file overlay.
//!
//! Split out of `file.rs` to keep that module under the 300-code-line cap
//! (Binding Rule 3). `file.rs` still owns `Config` itself and re-exports
//! [`RetrievalConfig`] so `comemory::config::file::RetrievalConfig` and
//! `cfg.retrieval.*` keep working unchanged.

use serde::{Deserialize, Serialize};

use super::defaults::{
    default_bm25_weights, default_code_bm25_weights, default_code_threshold,
    default_code_vector_dim, default_max_page_window, default_memory_vector_dim,
};

/// Hybrid-retrieval knobs consumed by `retrieval::{router,fuse,code_route}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalConfig {
    /// Minimum cosine similarity (`1.0 - distance`) for memory ANN hits.
    /// Consumed by the router's vector-consuming paths: KNN hits below
    /// this floor are dropped instead of padding the candidate pool with
    /// nearest-but-irrelevant noise. Default `0.55`.
    pub memory_threshold: f32,
    /// Minimum cosine similarity (`1.0 - distance`) for code ANN hits,
    /// the `code_vec` counterpart of [`memory_threshold`]. Must be a
    /// finite value in `[0.0, 1.0]`. Validated from day one; consumed by
    /// the M3 code-search wiring. Default `0.50`.
    #[serde(default = "default_code_threshold")]
    pub code_threshold: f32,
    pub hybrid_weight: f32,
    pub top_k: usize,
    /// Maximum depth a paginated retrieval (`search`, `search-code`,
    /// `context`) can page into the ranked result list. A request for
    /// `(offset, k)` fetches a candidate pool sized
    /// `clamp(offset + k + k, CANDIDATE_POOL, max_page_window)`, runs the
    /// full fuse → rerank → diversify pipeline over it, then slices the
    /// page; `has_more` is forced `false` once this window boundary is hit
    /// (deeper results require refining the query). Must be > 0.
    /// Default `200`.
    #[serde(default = "default_max_page_window")]
    pub max_page_window: usize,
    pub corrective_min_confidence: f32,
    /// RRF constant for sparse/dense fusion. Default 60.0 matches the original
    /// Cormack/Clarke/Buettcher RRF paper.
    pub rrf_k: f32,
    /// Maximum hop depth of the graph-expansion retrieval leg — the edge
    /// walk seeded from the provisional top hits. Must be <= 4; `0`
    /// disables the leg, so retrieval takes the legacy two-leg code path
    /// unchanged. Default `2`.
    pub graph_hops: u32,
    /// How many provisional top hits seed the graph-expansion walk. Must
    /// be >= 1. Default `8`.
    pub graph_seeds: usize,
    /// Weighted-BM25 column weights for `memory_fts` in column order
    /// `(body, tags)`. The `memory_id UNINDEXED` column is always 0.
    /// Both must be finite and >= 0, and at least one must be > 0.
    /// Default `(1.0, 3.0)` — a tag hit outranks a body hit.
    #[serde(default = "default_bm25_weights")]
    pub bm25_weights: (f32, f32),
    /// Weighted-BM25 column weights for `code_fts` in column order
    /// `(symbol, snippet, path_tokens)`. The `symbol_id UNINDEXED` column
    /// is always 0. Every weight must be finite and >= 0, and at least one
    /// must be > 0. Validated from day one; consumed by the M3 code-search
    /// wiring (which replaces the hardcoded
    /// `bm25(code_fts, 0.0, 2.0, 1.0, 1.5)`). Default `(2.0, 1.0, 1.5)` —
    /// a symbol-name hit outranks snippet and path hits.
    #[serde(default = "default_code_bm25_weights")]
    pub code_bm25_weights: (f32, f32, f32),
    /// Operator-visible record of the memory embedding dim. The authoritative
    /// value is the literal in `src/store/sql/0002_v2_tables.sql` —
    /// `memory_vec` is a vec0 vtab whose dim is baked into its `CREATE
    /// VIRTUAL TABLE` at migration time and cannot be changed afterwards.
    /// `vector::insert_memory` reads `schema_meta.memory_vector_dim` (seeded
    /// from the same migration) to gate inserts; this config field tracks
    /// the same value for `comemory doctor` reporting only. Changing it has
    /// no effect on the vtab and no env-var override is offered (a divergent
    /// env value would just produce `VecDimMismatch` at insert time).
    /// Defaults to 1024 (nomic-embed-text-v1.5).
    #[serde(default = "default_memory_vector_dim")]
    pub memory_vector_dim: usize,
    /// Operator-visible record of the code embedding dim. Same caveat as
    /// [`memory_vector_dim`]: authoritative value lives in the DDL, this
    /// field is reporting-only with no env override. Defaults to 768
    /// (jina-embeddings-v2-base-code).
    #[serde(default = "default_code_vector_dim")]
    pub code_vector_dim: usize,
}

/// File-overlay partial for [`RetrievalConfig`]. Only the M2-tunable
/// keys are overlayable; structural knobs (vector dims) stay
/// reporting-only and are not offered here.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartialRetrievalConfig {
    rrf_k: Option<f32>,
    bm25_weights: Option<(f32, f32)>,
    top_k: Option<usize>,
    max_page_window: Option<usize>,
    memory_threshold: Option<f32>,
    code_threshold: Option<f32>,
    code_bm25_weights: Option<(f32, f32, f32)>,
    graph_hops: Option<u32>,
    graph_seeds: Option<usize>,
}

impl RetrievalConfig {
    /// The shipped `[retrieval]` defaults, as built by `Config::defaults`.
    pub(crate) fn defaults() -> Self {
        Self {
            memory_threshold: 0.55,
            code_threshold: default_code_threshold(),
            hybrid_weight: 0.65,
            top_k: 12,
            max_page_window: default_max_page_window(),
            corrective_min_confidence: 0.15,
            rrf_k: 60.0,
            graph_hops: 2,
            graph_seeds: 8,
            bm25_weights: default_bm25_weights(),
            code_bm25_weights: default_code_bm25_weights(),
            memory_vector_dim: default_memory_vector_dim(),
            code_vector_dim: default_code_vector_dim(),
        }
    }

    /// Overlay the file's `[retrieval]` keys; absent keys leave `self`
    /// untouched.
    pub(crate) fn apply(&mut self, p: PartialRetrievalConfig) {
        if let Some(v) = p.rrf_k {
            self.rrf_k = v;
        }
        if let Some(v) = p.bm25_weights {
            self.bm25_weights = v;
        }
        if let Some(v) = p.top_k {
            self.top_k = v;
        }
        if let Some(v) = p.max_page_window {
            self.max_page_window = v;
        }
        if let Some(v) = p.memory_threshold {
            self.memory_threshold = v;
        }
        if let Some(v) = p.code_threshold {
            self.code_threshold = v;
        }
        if let Some(v) = p.code_bm25_weights {
            self.code_bm25_weights = v;
        }
        if let Some(v) = p.graph_hops {
            self.graph_hops = v;
        }
        if let Some(v) = p.graph_seeds {
            self.graph_seeds = v;
        }
    }
}
