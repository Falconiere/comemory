use serde::{Deserialize, Serialize};

use crate::prelude::*;

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
    pub memory_threshold: f32,
    pub code_threshold: f32,
    pub hybrid_weight: f32,
    pub top_k: usize,
    pub corrective_min_confidence: f32,
    /// RRF constant for sparse/dense fusion. Default 60.0 matches the original
    /// Cormack/Clarke/Buettcher RRF paper.
    pub rrf_k: f32,
    /// Operator-visible record of the memory embedding dim. The authoritative
    /// value is the literal in `src/store/sql/0002_v2_tables.sql` —
    /// `memory_vec` is a vec0 vtab whose dim is baked into its `CREATE
    /// VIRTUAL TABLE` at migration time and cannot be changed afterwards.
    /// `vector::insert_memory` reads `schema_meta.memory_vector_dim` (seeded
    /// from the same migration) to gate inserts, so this config field is
    /// surfaced for reporting / drift detection only; setting
    /// `COMEMORY_VECTOR_DIM` does NOT reshape the vtab. Defaults to 1024
    /// (nomic-embed-text-v1.5).
    #[serde(default = "default_memory_vector_dim")]
    pub memory_vector_dim: usize,
    /// Operator-visible record of the code embedding dim. Same caveat as
    /// [`memory_vector_dim`]: the authoritative value is the literal in
    /// `0002_v2_tables.sql` and cannot be reshaped via env after the first
    /// migration runs. Defaults to 768 (jina-embeddings-v2-base-code).
    #[serde(default = "default_code_vector_dim")]
    pub code_vector_dim: usize,
}

fn default_memory_vector_dim() -> usize {
    1024
}

fn default_code_vector_dim() -> usize {
    768
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneConfig {
    pub trash_retention_days: u32,
    pub low_value_default_unused_since_days: u32,
    pub low_value_default_below_quality: u32,
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
                code_threshold: 0.50,
                hybrid_weight: 0.65,
                top_k: 12,
                corrective_min_confidence: 0.15,
                rrf_k: 60.0,
                memory_vector_dim: default_memory_vector_dim(),
                code_vector_dim: default_code_vector_dim(),
            },
            prune: PruneConfig {
                trash_retention_days: 30,
                low_value_default_unused_since_days: 180,
                low_value_default_below_quality: 2,
            },
            output: OutputConfig {
                json: false,
                color: "auto".into(),
            },
            embed_hint: None,
        }
    }

    /// Apply `COMEMORY_*` env-var overrides on top of `self`.
    ///
    /// Unlike the previous infallible variant, parse failures (non-numeric
    /// `top_k` / thresholds, unknown `auto_reindex` mode, unknown boolean for
    /// `auto_sync`) are now surfaced as `Error::Other` rather than silently
    /// dropped. This catches typos at startup instead of letting them mask as
    /// "defaults applied".
    pub fn with_env(mut self) -> Result<Self> {
        if let Ok(v) = std::env::var("COMEMORY_INDEXING_AUTO_REINDEX") {
            self.indexing.auto_reindex = match v.as_str() {
                "lazy" => AutoReindexMode::Lazy,
                "hook" => AutoReindexMode::Hook,
                "off" => AutoReindexMode::Off,
                other => {
                    return Err(Error::Other(format!(
                        "invalid env var COMEMORY_INDEXING_AUTO_REINDEX: '{other}' (expected lazy|hook|off)"
                    )));
                }
            };
        }
        if let Ok(v) = std::env::var("COMEMORY_RETRIEVAL_TOP_K") {
            self.retrieval.top_k = v.parse::<usize>().map_err(|e| {
                Error::Other(format!("invalid env var COMEMORY_RETRIEVAL_TOP_K: {e}"))
            })?;
        }
        if let Ok(v) = std::env::var("COMEMORY_RETRIEVAL_MEMORY_THRESHOLD") {
            self.retrieval.memory_threshold = v.parse::<f32>().map_err(|e| {
                Error::Other(format!(
                    "invalid env var COMEMORY_RETRIEVAL_MEMORY_THRESHOLD: {e}"
                ))
            })?;
        }
        if let Ok(v) = std::env::var("COMEMORY_RETRIEVAL_CODE_THRESHOLD") {
            self.retrieval.code_threshold = v.parse::<f32>().map_err(|e| {
                Error::Other(format!(
                    "invalid env var COMEMORY_RETRIEVAL_CODE_THRESHOLD: {e}"
                ))
            })?;
        }
        if let Ok(v) = std::env::var("COMEMORY_RETRIEVAL_RRF_K") {
            let parsed = v.parse::<f32>().map_err(|e| {
                Error::Other(format!("invalid env var COMEMORY_RETRIEVAL_RRF_K: {e}"))
            })?;
            if !parsed.is_finite() || parsed <= 0.0 {
                return Err(Error::Other(format!(
                    "invalid env var COMEMORY_RETRIEVAL_RRF_K={v} must be a finite positive number"
                )));
            }
            self.retrieval.rrf_k = parsed;
        }
        if let Ok(v) = std::env::var("COMEMORY_GIT_AUTO_SYNC") {
            self.git.auto_sync = match v.as_str() {
                "true" | "1" | "yes" | "on" => true,
                "false" | "0" | "no" | "off" => false,
                other => {
                    return Err(Error::Other(format!(
                        "invalid env var COMEMORY_GIT_AUTO_SYNC: '{other}' (expected true|1|yes|on or false|0|no|off)"
                    )));
                }
            };
        }
        if let Ok(v) = std::env::var("COMEMORY_VECTOR_DIM") {
            let parsed = v
                .parse::<usize>()
                .map_err(|e| Error::Config(format!("invalid env var COMEMORY_VECTOR_DIM: {e}")))?;
            if parsed == 0 {
                return Err(Error::Config(
                    "invalid env var COMEMORY_VECTOR_DIM=0: must be a positive integer".into(),
                ));
            }
            self.retrieval.memory_vector_dim = parsed;
        }
        if let Ok(v) = std::env::var("COMEMORY_CODE_VECTOR_DIM") {
            let parsed = v.parse::<usize>().map_err(|e| {
                Error::Config(format!("invalid env var COMEMORY_CODE_VECTOR_DIM: {e}"))
            })?;
            if parsed == 0 {
                return Err(Error::Config(
                    "invalid env var COMEMORY_CODE_VECTOR_DIM=0: must be a positive integer".into(),
                ));
            }
            self.retrieval.code_vector_dim = parsed;
        }
        if let Ok(v) = std::env::var("COMEMORY_EMBED_HINT") {
            self.embed_hint = Some(v);
        }
        Ok(self)
    }
}
