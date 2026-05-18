use serde::{Deserialize, Serialize};

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
        }
    }

    pub fn with_env(mut self) -> Self {
        if let Ok(v) = std::env::var("QWICK_INDEXING_AUTO_REINDEX") {
            self.indexing.auto_reindex = match v.as_str() {
                "hook" => AutoReindexMode::Hook,
                "off" => AutoReindexMode::Off,
                _ => AutoReindexMode::Lazy,
            };
        }
        if let Ok(v) = std::env::var("QWICK_RETRIEVAL_TOP_K") {
            if let Ok(n) = v.parse() {
                self.retrieval.top_k = n;
            }
        }
        if let Ok(v) = std::env::var("QWICK_RETRIEVAL_MEMORY_THRESHOLD") {
            if let Ok(n) = v.parse() {
                self.retrieval.memory_threshold = n;
            }
        }
        if let Ok(v) = std::env::var("QWICK_RETRIEVAL_CODE_THRESHOLD") {
            if let Ok(n) = v.parse() {
                self.retrieval.code_threshold = n;
            }
        }
        if let Ok(v) = std::env::var("QWICK_GIT_AUTO_SYNC") {
            self.git.auto_sync = v == "true" || v == "1";
        }
        self
    }
}
