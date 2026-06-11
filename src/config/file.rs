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
}

/// File-overlay partial for [`RankConfig`]. All fields optional.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PartialRankConfig {
    decay: Option<f64>,
    prior_clamp: Option<(f64, f64)>,
    mmr_lambda: Option<f64>,
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

fn default_code_vector_dim() -> usize {
    768
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
                hybrid_weight: 0.65,
                top_k: 12,
                corrective_min_confidence: 0.15,
                rrf_k: 60.0,
                bm25_weights: default_bm25_weights(),
                memory_vector_dim: default_memory_vector_dim(),
                code_vector_dim: default_code_vector_dim(),
            },
            rank: RankConfig {
                decay: 0.5,
                prior_clamp: (0.5, 2.0),
                mmr_lambda: 0.7,
            },
            prune: PruneConfig {
                trash_retention_days: 30,
                low_value_default_below_quality: 2,
                min_activation: -2.0,
                min_feedback: 0.25,
            },
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
        }
        self.validate()
    }

    /// Enforce the documented rank/prune invariants.
    ///
    /// Runs at the end of both [`Config::with_file`] and
    /// [`Config::with_env`] so the file overlay and env overrides are
    /// validated identically — `[rank] decay = -1.0` in config.toml fails
    /// exactly like `COMEMORY_RANK_DECAY=-1.0`. Each message names both
    /// the config field and its env var so the offending knob is
    /// identifiable from either entry point.
    pub(crate) fn validate(self) -> Result<Self> {
        let (b, t) = self.retrieval.bm25_weights;
        if !b.is_finite() || !t.is_finite() || b < 0.0 || t < 0.0 || (b == 0.0 && t == 0.0) {
            return Err(Error::Config(format!(
                "invalid retrieval.bm25_weights={b},{t} (env COMEMORY_RETRIEVAL_BM25_WEIGHTS): both must be finite and >= 0, and at least one > 0"
            )));
        }
        let k = self.retrieval.rrf_k;
        if !k.is_finite() || k <= 0.0 {
            return Err(Error::Config(format!(
                "invalid retrieval.rrf_k={k} (env COMEMORY_RETRIEVAL_RRF_K): must be a finite positive number"
            )));
        }
        let d = self.rank.decay;
        if !d.is_finite() || d < 0.0 {
            return Err(Error::Config(format!(
                "invalid rank.decay={d} (env COMEMORY_RANK_DECAY): must be a finite non-negative number"
            )));
        }
        let (lo, hi) = self.rank.prior_clamp;
        if !lo.is_finite() || !hi.is_finite() || lo <= 0.0 || lo > hi {
            return Err(Error::Config(format!(
                "invalid rank.prior_clamp={lo},{hi} (env COMEMORY_RANK_PRIOR_CLAMP): both values must be finite, lo > 0, and lo <= hi"
            )));
        }
        let l = self.rank.mmr_lambda;
        if !l.is_finite() || !(0.0..=1.0).contains(&l) {
            return Err(Error::Config(format!(
                "invalid rank.mmr_lambda={l} (env COMEMORY_RANK_MMR_LAMBDA): must be a finite value in [0.0, 1.0]"
            )));
        }
        let a = self.prune.min_activation;
        if !a.is_finite() {
            return Err(Error::Config(format!(
                "invalid prune.min_activation={a} (env COMEMORY_PRUNE_MIN_ACTIVATION): must be a finite number"
            )));
        }
        let f = self.prune.min_feedback;
        if !f.is_finite() || !(0.0..=1.0).contains(&f) {
            return Err(Error::Config(format!(
                "invalid prune.min_feedback={f} (env COMEMORY_PRUNE_MIN_FEEDBACK): must be a finite value in [0.0, 1.0]"
            )));
        }
        let q = self.prune.low_value_default_below_quality;
        if !(1..=5).contains(&q) {
            return Err(Error::Config(format!(
                "invalid prune.low_value_default_below_quality={q} (env COMEMORY_PRUNE_BELOW_QUALITY): must be in 1..=5"
            )));
        }
        Ok(self)
    }
}
