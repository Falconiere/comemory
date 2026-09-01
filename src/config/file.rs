use std::path::Path;

use serde::{Deserialize, Serialize};

use super::defaults::{
    default_max_file_bytes, default_near_dup_hamming, default_superseded_grace_days,
};
use super::learning::{
    BanditConfig, PartialBanditConfig, PartialReinforceConfig, PartialTuneConfig, ReinforceConfig,
};
use super::retrieval::PartialRetrievalConfig;
use crate::prelude::*;

/// Re-export: historical `config::file::TuneConfig` import path.
pub use super::learning::TuneConfig;
/// Re-export: historical `config::file::RetrievalConfig` import path.
pub use super::retrieval::RetrievalConfig;

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
    /// Search→edit lookback. Absent keys leave defaults.
    reinforce: Option<PartialReinforceConfig>,
    /// Bandit apply gate. Absent keys leave defaults.
    bandit: Option<PartialBanditConfig>,
    /// Optional file-overlay for document-source indexing knobs. Absent
    /// keys leave defaults.
    indexing: Option<PartialIndexingConfig>,
}

/// File-overlay partial for [`IndexingConfig`]. Carries every field (not
/// just the new `max_file_bytes` key) so `deny_unknown_fields` cannot
/// hard-error on a legitimate `[indexing]` key once the section is
/// overlayable at all — same rationale as [`PartialPruneConfig`].
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PartialIndexingConfig {
    auto_reindex: Option<AutoReindexMode>,
    auto_reindex_threshold_ms: Option<u64>,
    incremental_batch_size: Option<usize>,
    max_file_bytes: Option<u64>,
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

/// How the code index is refreshed — see [`IndexingConfig::auto_reindex`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AutoReindexMode {
    /// `search-code` / `context` spawn a detached background `index-code`.
    Lazy,
    /// Refresh relies on the installed git hooks; no in-process trigger.
    Hook,
    /// Manual only: the index refreshes on an explicit `index-code`.
    Off,
}

/// Best-effort git auto-sync of the markdown source of truth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    /// Commit + push after a save when enabled. Env: `COMEMORY_GIT_AUTO_SYNC`.
    pub auto_sync: bool,
    /// Remote pushed to by the auto-sync; empty means the git default.
    pub remote: String,
}

/// Operator-visible record of the embedders that produced the vectors.
///
/// Reporting-only: comemory is BYO-vector and never runs an embedder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsConfig {
    /// Model name recorded for `memory_vec` vectors.
    pub memory_model: String,
    /// Model name recorded for `code_vec` vectors.
    pub code_model: String,
}

/// Code-index freshness knobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingConfig {
    /// How the code index is kept fresh: `lazy` (default), `hook`, or `off`.
    ///
    /// `lazy` — `search-code` / `context` fire a DETACHED background
    /// `index-code` when the repo's HEAD has moved since the last index, then
    /// search the current index immediately (see `cli::lazy_reindex`).
    /// `hook` — relies on installed git hooks; no in-process trigger. `off` —
    /// the index refreshes only on a manual `index-code`.
    pub auto_reindex: AutoReindexMode,
    /// Debounce window (milliseconds) for the `lazy` background trigger.
    ///
    /// After a lazy reindex is fired for a repo, a fresh spawn is suppressed
    /// for this many milliseconds even across a HEAD change, so a burst of
    /// searches (e.g. during a rebase) cannot fork a herd of `index-code`
    /// processes. A trigger already fired for the *current* HEAD is always
    /// suppressed regardless of this window. Default `200`.
    pub auto_reindex_threshold_ms: u64,
    /// RESERVED. Intended max files per incremental indexing batch.
    ///
    /// Currently parsed and validated but not consumed: `index-code` runs the
    /// whole walk in one SQLite transaction with no batch seam to thread this
    /// through, and the `lazy` trigger delegates to that single `index-code`
    /// invocation. Kept as an honest reserved knob (rather than wired to a
    /// fake consumer) for a future chunked-commit indexing path. Default `50`.
    pub incremental_batch_size: usize,
    /// Ceiling (bytes) above which a candidate document file is recorded
    /// `too_large` and skipped by the document index writer
    /// (`comemory index`) instead of extracted — plain-text formats past
    /// this size are logs, not documents. Must be > 0. Default
    /// `16_777_216` (16 MiB). Env: `COMEMORY_INDEXING_MAX_FILE_BYTES`.
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: u64,
}

impl IndexingConfig {
    /// Overlay the file's `[indexing]` keys; absent keys leave `self`
    /// untouched.
    fn apply(&mut self, p: PartialIndexingConfig) {
        if let Some(v) = p.auto_reindex {
            self.auto_reindex = v;
        }
        if let Some(v) = p.auto_reindex_threshold_ms {
            self.auto_reindex_threshold_ms = v;
        }
        if let Some(v) = p.incremental_batch_size {
            self.incremental_batch_size = v;
        }
        if let Some(v) = p.max_file_bytes {
            self.max_file_bytes = v;
        }
    }
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

impl RankConfig {
    /// Overlay the file's `[rank]` keys; absent keys leave `self` untouched.
    fn apply(&mut self, p: PartialRankConfig) {
        if let Some(v) = p.decay {
            self.decay = v;
        }
        if let Some(v) = p.prior_clamp {
            self.prior_clamp = v;
        }
        if let Some(v) = p.mmr_lambda {
            self.mmr_lambda = v;
        }
        if let Some(v) = p.near_dup_hamming {
            self.near_dup_hamming = v;
        }
    }
}

/// Prune scoring floors and retention windows for `comemory prune` / `gc`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneConfig {
    /// Days a soft-deleted memory stays in the trash before `gc` reaps it.
    pub trash_retention_days: u32,
    /// Quality (1..=5) at or below which a memory is a low-value candidate.
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

impl PruneConfig {
    /// Overlay the file's `[prune]` keys; absent keys leave `self` untouched.
    fn apply(&mut self, p: PartialPruneConfig) {
        if let Some(v) = p.trash_retention_days {
            self.trash_retention_days = v;
        }
        if let Some(v) = p.low_value_default_below_quality {
            self.low_value_default_below_quality = v;
        }
        if let Some(v) = p.min_activation {
            self.min_activation = v;
        }
        if let Some(v) = p.min_feedback {
            self.min_feedback = v;
        }
        if let Some(v) = p.learning_retention_days {
            self.learning_retention_days = v;
        }
        if let Some(v) = p.superseded_grace_days {
            self.superseded_grace_days = v;
        }
    }
}

/// Emitter defaults shared by every subcommand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    /// Emit JSON instead of the TTY renderer. Overridden by `--json`.
    pub json: bool,
    /// Colour policy for the TTY renderer: `auto`, `always`, or `never`.
    pub color: String,
}

/// The fully-layered configuration: defaults → `config.toml` → env.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Git auto-sync knobs.
    pub git: GitConfig,
    /// Reporting-only record of the active embedders.
    pub embeddings: EmbeddingsConfig,
    /// Code-index freshness knobs.
    pub indexing: IndexingConfig,
    /// Hybrid-retrieval knobs — see [`RetrievalConfig`].
    pub retrieval: RetrievalConfig,
    /// Rerank/diversify knobs — see [`RankConfig`].
    pub rank: RankConfig,
    /// Prune scoring floors and retention windows — see [`PruneConfig`].
    pub prune: PruneConfig,
    /// Grid lists for `comemory tune`. File-only — see [`TuneConfig`].
    #[serde(default)]
    pub tune: TuneConfig,
    /// Search→edit auto-reinforcement lookback.
    #[serde(default)]
    pub reinforce: ReinforceConfig,
    /// `comemory bandit` apply gate.
    #[serde(default)]
    pub bandit: BanditConfig,
    /// Emitter defaults — see [`OutputConfig`].
    pub output: OutputConfig,
    /// Free-form caller-set hint identifying the embedder that produced the
    /// vectors (e.g. `ollama:nomic-embed-text`). Surfaced verbatim by
    /// `comemory doctor`; comemory itself never reads it as a switch.
    #[serde(default)]
    pub embed_hint: Option<String>,
}

impl Config {
    /// The shipped defaults — the base layer every overlay is applied to.
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
                max_file_bytes: default_max_file_bytes(),
            },
            retrieval: RetrievalConfig::defaults(),
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
            reinforce: ReinforceConfig::default(),
            bandit: BanditConfig::default(),
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
            self.retrieval.apply(pr);
        }
        if let Some(pr) = partial.rank {
            self.rank.apply(pr);
        }
        if let Some(pp) = partial.prune {
            self.prune.apply(pp);
        }
        if let Some(pt) = partial.tune {
            self.tune.apply(pt);
        }
        if let Some(pr) = partial.reinforce {
            if let Some(v) = pr.search_edit_days {
                self.reinforce.search_edit_days = v;
            }
            if let Some(v) = pr.enabled {
                self.reinforce.enabled = v;
            }
        }
        if let Some(pb) = partial.bandit
            && let Some(v) = pb.enabled
        {
            self.bandit.enabled = v;
        }
        if let Some(pi) = partial.indexing {
            self.indexing.apply(pi);
        }
        self.validate()
    }
}

#[cfg(test)]
#[path = "tests/file.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/file_2.rs"]
mod tests_2;
