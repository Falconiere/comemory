use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::prelude::*;

/// Partial config overlay loaded from a `config.toml` file.
///
/// Every field is `Option<_>` so a sparse file that only sets a handful of
/// keys is valid TOML. Fields present in the file overlay the defaults;
/// absent fields leave the defaults untouched. Env-var overrides applied
/// afterwards via [`Config::with_env`] take precedence over the file.
///
/// `deny_unknown_fields` makes a typo in a config key (e.g. `embedhint`
/// instead of `embed_hint`) a hard error at load time rather than
/// silently dropping the override and leaving the operator wondering why
/// their setting didn't take effect.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PartialConfig {
    /// Operator-supplied hint identifying the active embedder; surfaced by
    /// `comemory doctor` and echoed back verbatim. Not interpreted by
    /// comemory itself.
    embed_hint: Option<String>,
    /// Optional file-overlay for retrieval knobs. Absent keys leave defaults.
    retrieval: Option<PartialRetrievalConfig>,
    /// Optional file-overlay for ranking knobs. Absent keys leave defaults.
    rank: Option<PartialRankConfig>,
    /// Optional file-overlay for prune scoring knobs. Absent keys leave defaults.
    prune: Option<PartialPruneConfig>,
    /// Optional file-overlay for the `comemory tune` grid lists. Absent keys
    /// leave defaults. File-only: no env-var equivalents exist (a four-list
    /// env value is unreadable).
    tune: Option<PartialTuneConfig>,
}

/// File-overlay partial for [`RetrievalConfig`]. Only the M2-tunable
/// keys are overlayable; structural knobs (vector dims) stay
/// reporting-only and are not offered here.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PartialRetrievalConfig {
    rrf_k: Option<f32>,
    bm25_weights: Option<(f32, f32)>,
    top_k: Option<usize>,
    memory_threshold: Option<f32>,
    code_threshold: Option<f32>,
    code_bm25_weights: Option<(f32, f32, f32)>,
}

/// File-overlay partial for [`RankConfig`]. All fields optional.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PartialRankConfig {
    decay: Option<f64>,
    prior_clamp: Option<(f64, f64)>,
    mmr_lambda: Option<f64>,
    near_dup_hamming: Option<u32>,
}

/// File-overlay partial for [`TuneConfig`]. All fields optional.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PartialTuneConfig {
    rrf_k_grid: Option<Vec<f32>>,
    decay_grid: Option<Vec<f64>>,
    mmr_lambda_grid: Option<Vec<f64>>,
    bm25_grid: Option<Vec<(f32, f32)>>,
}

/// File-overlay partial for [`PruneConfig`]. All fields optional.
///
/// Carries every *consumed* `PruneConfig` field, not just the M1 scoring
/// extensions: `deny_unknown_fields` would otherwise hard-error on a valid
/// `[prune]` key like `trash_retention_days` once the section is
/// overlayable at all.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PartialPruneConfig {
    trash_retention_days: Option<u32>,
    low_value_default_below_quality: Option<u32>,
    min_activation: Option<f64>,
    min_feedback: Option<f64>,
    learning_retention_days: Option<u32>,
    superseded_grace_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AutoReindexMode {
    Lazy,
    Hook,
    Off,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    pub auto_sync: bool,
    pub remote: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsConfig {
    pub memory_model: String,
    pub code_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingConfig {
    pub auto_reindex: AutoReindexMode,
    pub auto_reindex_threshold_ms: u64,
    pub incremental_batch_size: usize,
}

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
    pub corrective_min_confidence: f32,
    /// RRF constant for sparse/dense fusion. Default 60.0 matches the original
    /// Cormack/Clarke/Buettcher RRF paper.
    pub rrf_k: f32,
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

fn default_memory_vector_dim() -> usize {
    1024
}

fn default_bm25_weights() -> (f32, f32) {
    (1.0, 3.0)
}

fn default_code_threshold() -> f32 {
    0.50
}

fn default_code_bm25_weights() -> (f32, f32, f32) {
    (2.0, 1.0, 1.5)
}

fn default_code_vector_dim() -> usize {
    768
}

/// The shared constant in `simhash` stays the single source of the default
/// radius; the config field merely makes it operator-tunable.
fn default_near_dup_hamming() -> u32 {
    crate::simhash::NEAR_DUP_HAMMING
}

/// The constant next to the prune rule stays the single source of the
/// default grace window; the config field merely makes it operator-tunable.
fn default_superseded_grace_days() -> u32 {
    crate::prune::low_value::SUPERSEDED_GRACE_DAYS
}

/// Ranking knobs for the rerank/diversify pipeline (M1).
///
/// These values are consumed by `retrieval::{rerank,diversify}` and the
/// ACT-R decay scorer.
/// Defaults are tuned for typical developer-memory workloads; operators can
/// override via env vars or a `[rank]` section in `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankConfig {
    /// ACT-R decay exponent `d` in `ln(n) − d·ln(days + 1)`.
    ///
    /// Must be ≥ 0. Higher values decay older memories faster.
    /// Default: `0.5` (moderate recency preference).
    pub decay: f64,
    /// Bounds `(lo, hi)` applied to every rerank prior multiplier.
    ///
    /// Both values must be finite; `lo` must be > 0 and ≤ `hi`.
    /// Default: `(0.5, 2.0)`.
    pub prior_clamp: (f64, f64),
    /// MMR relevance-vs-diversity trade-off in `[0.0, 1.0]`.
    ///
    /// `1.0` = pure relevance (no diversification); `0.0` = pure diversity.
    /// Default: `0.7` (lean toward relevance).
    pub mmr_lambda: f64,
    /// SimHash Hamming radius treated as "same memory, different wording".
    ///
    /// Consumed by the query-time near-duplicate collapse
    /// (`retrieval::diversify`) and the save-time duplicate warning. Must
    /// be <= 64 (the hash is 64-bit; a larger radius would collapse every
    /// pair); `0` collapses only bit-identical hashes.
    /// Default: `crate::simhash::NEAR_DUP_HAMMING` (8).
    #[serde(default = "default_near_dup_hamming")]
    pub near_dup_hamming: u32,
}

/// Grid lists for `comemory tune`'s deterministic search — the cartesian
/// product of the four lists is the candidate grid.
///
/// File-only (`[tune]` in `config.toml`): no `COMEMORY_TUNE_*` env vars are
/// offered because a four-list env value is unreadable and error-prone.
/// Each list must be non-empty, and every value must pass the same bounds
/// its scalar knob enforces (see `Config::validate`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuneConfig {
    /// RRF fusion constants to sweep; same finite-positive invariant as
    /// `retrieval.rrf_k`. Default `[20.0, 60.0, 100.0]`.
    pub rrf_k_grid: Vec<f32>,
    /// ACT-R decay exponents to sweep; same finite, >= 0 invariant as
    /// `rank.decay`. Default `[0.3, 0.5, 0.8]`.
    pub decay_grid: Vec<f64>,
    /// MMR lambdas to sweep; same `[0.0, 1.0]` invariant as
    /// `rank.mmr_lambda`. Default `[0.5, 0.7, 0.9]`.
    pub mmr_lambda_grid: Vec<f64>,
    /// `(body, tags)` BM25 weight pairs to sweep; same invariants as
    /// `retrieval.bm25_weights`. Default `[(1.0, 3.0), (1.0, 1.0), (2.0, 1.0)]`.
    pub bm25_grid: Vec<(f32, f32)>,
}

impl Default for TuneConfig {
    /// The M1 3×3×3×3 grid (81 points), bracketing the shipped defaults.
    fn default() -> Self {
        Self {
            rrf_k_grid: vec![20.0, 60.0, 100.0],
            decay_grid: vec![0.3, 0.5, 0.8],
            mmr_lambda_grid: vec![0.5, 0.7, 0.9],
            bm25_grid: vec![(1.0, 3.0), (1.0, 1.0), (2.0, 1.0)],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneConfig {
    pub trash_retention_days: u32,
    pub low_value_default_below_quality: u32,
    /// Activation floor (ACT-R scale) below which a memory is prune-eligible.
    ///
    /// Memories whose computed activation falls below this threshold are
    /// candidates for soft-deletion. Default: `-2.0`.
    pub min_activation: f64,
    /// Beta-feedback ceiling at or below which a memory is prune-eligible.
    ///
    /// Range `[0.0, 1.0]`. A memory with cumulative feedback ≤ this value
    /// is considered low-value. Default: `0.25`.
    pub min_feedback: f64,
    /// Days to retain learning telemetry (`retrieval_log` rows and
    /// `feedback_events` rows). `comemory gc` deletes older rows.
    /// Aggregated `feedback` counters are permanent — only raw event
    /// rows age out. Must be >= 1. Default: `90`.
    pub learning_retention_days: u32,
    /// Grace window (days) for the superseded-and-forgotten prune rule:
    /// only supersede edges older than this many days count. Protects
    /// freshly-rebuilt DBs, whose edges all carry rebuild-time timestamps.
    /// `0` disables the grace entirely.
    /// Default: `crate::prune::low_value::SUPERSEDED_GRACE_DAYS` (7).
    #[serde(default = "default_superseded_grace_days")]
    pub superseded_grace_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    pub json: bool,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub git: GitConfig,
    pub embeddings: EmbeddingsConfig,
    pub indexing: IndexingConfig,
    pub retrieval: RetrievalConfig,
    pub rank: RankConfig,
    pub prune: PruneConfig,
    /// Grid lists for `comemory tune`. File-only — see [`TuneConfig`].
    #[serde(default)]
    pub tune: TuneConfig,
    pub output: OutputConfig,
    /// Free-form caller-set hint identifying the embedder that produced the
    /// vectors (e.g. `ollama:nomic-embed-text`). Surfaced verbatim by
    /// `comemory doctor`; comemory itself never reads it as a switch.
    #[serde(default)]
    pub embed_hint: Option<String>,
}

impl Config {
    pub fn defaults() -> Self {
        Self {
            git: GitConfig {
                auto_sync: false,
                remote: String::new(),
            },
            embeddings: EmbeddingsConfig {
                memory_model: "nomic-embed-text-v1.5-Q".into(),
                code_model: "jina-embeddings-v2-base-code-Q".into(),
            },
            indexing: IndexingConfig {
                auto_reindex: AutoReindexMode::Lazy,
                auto_reindex_threshold_ms: 200,
                incremental_batch_size: 50,
            },
            retrieval: RetrievalConfig {
                memory_threshold: 0.55,
                code_threshold: default_code_threshold(),
                hybrid_weight: 0.65,
                top_k: 12,
                corrective_min_confidence: 0.15,
                rrf_k: 60.0,
                bm25_weights: default_bm25_weights(),
                code_bm25_weights: default_code_bm25_weights(),
                memory_vector_dim: default_memory_vector_dim(),
                code_vector_dim: default_code_vector_dim(),
            },
            rank: RankConfig {
                decay: 0.5,
                prior_clamp: (0.5, 2.0),
                mmr_lambda: 0.7,
                near_dup_hamming: default_near_dup_hamming(),
            },
            prune: PruneConfig {
                trash_retention_days: 30,
                low_value_default_below_quality: 2,
                min_activation: -2.0,
                min_feedback: 0.25,
                learning_retention_days: 90,
                superseded_grace_days: default_superseded_grace_days(),
            },
            tune: TuneConfig::default(),
            output: OutputConfig {
                json: false,
                color: "auto".into(),
            },
            embed_hint: None,
        }
    }

    /// Overlay settings from an optional TOML config file.
    ///
    /// Only keys present in the file are applied; absent keys leave the
    /// defaults (or any previously-applied overrides) untouched. Returns
    /// `self` unchanged when the file does not exist, so callers can call
    /// `Config::defaults().with_file(path)?.with_env()?` unconditionally.
    ///
    /// A file that exists but fails to parse is a hard error so operators
    /// notice immediately rather than silently running on stale defaults.
    pub fn with_file(mut self, path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(self);
        }
        let raw = std::fs::read_to_string(path).map_err(Error::Io)?;
        let partial: PartialConfig =
            toml::from_str(&raw).map_err(|e| Error::Config(format!("config.toml: {e}")))?;
        if let Some(hint) = partial.embed_hint {
            self.embed_hint = Some(hint);
        }
        if let Some(pr) = partial.retrieval {
            if let Some(v) = pr.rrf_k {
                self.retrieval.rrf_k = v;
            }
            if let Some(v) = pr.bm25_weights {
                self.retrieval.bm25_weights = v;
            }
            if let Some(v) = pr.top_k {
                self.retrieval.top_k = v;
            }
            if let Some(v) = pr.memory_threshold {
                self.retrieval.memory_threshold = v;
            }
            if let Some(v) = pr.code_threshold {
                self.retrieval.code_threshold = v;
            }
            if let Some(v) = pr.code_bm25_weights {
                self.retrieval.code_bm25_weights = v;
            }
        }
        if let Some(pr) = partial.rank {
            if let Some(v) = pr.decay {
                self.rank.decay = v;
            }
            if let Some(v) = pr.prior_clamp {
                self.rank.prior_clamp = v;
            }
            if let Some(v) = pr.mmr_lambda {
                self.rank.mmr_lambda = v;
            }
            if let Some(v) = pr.near_dup_hamming {
                self.rank.near_dup_hamming = v;
            }
        }
        if let Some(pp) = partial.prune {
            if let Some(v) = pp.trash_retention_days {
                self.prune.trash_retention_days = v;
            }
            if let Some(v) = pp.low_value_default_below_quality {
                self.prune.low_value_default_below_quality = v;
            }
            if let Some(v) = pp.min_activation {
                self.prune.min_activation = v;
            }
            if let Some(v) = pp.min_feedback {
                self.prune.min_feedback = v;
            }
            if let Some(v) = pp.learning_retention_days {
                self.prune.learning_retention_days = v;
            }
            if let Some(v) = pp.superseded_grace_days {
                self.prune.superseded_grace_days = v;
            }
        }
        if let Some(pt) = partial.tune {
            if let Some(v) = pt.rrf_k_grid {
                self.tune.rrf_k_grid = v;
            }
            if let Some(v) = pt.decay_grid {
                self.tune.decay_grid = v;
            }
            if let Some(v) = pt.mmr_lambda_grid {
                self.tune.mmr_lambda_grid = v;
            }
            if let Some(v) = pt.bm25_grid {
                self.tune.bm25_grid = v;
            }
        }
        self.validate()
    }
}
