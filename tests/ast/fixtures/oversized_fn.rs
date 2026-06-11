// Real oversized function copied from THIS repo:
// src/config/env.rs::Config::with_env (88 lines at copy time).
// Adapted only by dropping the `pub ` qualifier so the extractor's
// plain-`fn` pattern matches it; the body is verbatim. The fixture is
// parsed by tree-sitter, never compiled, so the bare `self` receiver
// and unresolved names are fine.
fn with_env(mut self) -> Result<Self> {
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
    if let Some(v) = env_parse::<usize>("COMEMORY_RETRIEVAL_TOP_K")? {
        self.retrieval.top_k = v;
    }
    if let Some(v) = env_parse::<f32>("COMEMORY_RETRIEVAL_MEMORY_THRESHOLD")? {
        self.retrieval.memory_threshold = v;
    }
    // Only the parse happens here; the finite/positive invariant lives
    // in `Config::validate` so the file overlay is checked identically.
    if let Some(v) = env_parse::<f32>("COMEMORY_RETRIEVAL_RRF_K")? {
        self.retrieval.rrf_k = v;
    }
    if let Some(v) = env_pair::<f32>("COMEMORY_RETRIEVAL_BM25_WEIGHTS")? {
        self.retrieval.bm25_weights = v;
    }
    if let Some(v) = env_parse::<f32>("COMEMORY_RETRIEVAL_CODE_THRESHOLD")? {
        self.retrieval.code_threshold = v;
    }
    if let Some(v) = env_triple::<f32>("COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS")? {
        self.retrieval.code_bm25_weights = v;
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
    // COMEMORY_VECTOR_DIM and COMEMORY_CODE_VECTOR_DIM are intentionally
    // not honoured here. The authoritative dim lives in the `memory_vec`
    // / `code_vec` vec0 DDL (`src/store/sql/0002_v2_tables.sql`) and is
    // baked in at migration time; an env override would silently disagree
    // with the vtab and surface as `VecDimMismatch` at first insert.
    if let Ok(v) = std::env::var("COMEMORY_EMBED_HINT") {
        self.embed_hint = Some(v);
    }
    // ── Rank + prune knobs ───────────────────────────────────────────────
    // Parsing happens here; range invariants are enforced once for both
    // env and file overlays by `Config::validate`.
    if let Some(v) = env_parse::<f64>("COMEMORY_RANK_DECAY")? {
        self.rank.decay = v;
    }
    if let Some(v) = env_pair::<f64>("COMEMORY_RANK_PRIOR_CLAMP")? {
        self.rank.prior_clamp = v;
    }
    if let Some(v) = env_parse::<f64>("COMEMORY_RANK_MMR_LAMBDA")? {
        self.rank.mmr_lambda = v;
    }
    if let Some(v) = env_parse::<u32>("COMEMORY_RANK_NEAR_DUP_HAMMING")? {
        self.rank.near_dup_hamming = v;
    }
    if let Some(v) = env_parse::<f64>("COMEMORY_PRUNE_MIN_ACTIVATION")? {
        self.prune.min_activation = v;
    }
    if let Some(v) = env_parse::<f64>("COMEMORY_PRUNE_MIN_FEEDBACK")? {
        self.prune.min_feedback = v;
    }
    if let Some(v) = env_parse::<u32>("COMEMORY_PRUNE_BELOW_QUALITY")? {
        self.prune.low_value_default_below_quality = v;
    }
    if let Some(v) = env_parse::<u32>("COMEMORY_LEARNING_RETENTION_DAYS")? {
        self.prune.learning_retention_days = v;
    }
    if let Some(v) = env_parse::<u32>("COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS")? {
        self.prune.superseded_grace_days = v;
    }
    // The `[tune]` grid lists deliberately have NO env equivalents: a
    // four-list env value ("20,60,100" × 4 vars, or worse, one var with
    // semicolons) is unreadable and easy to misquote. Set them in
    // config.toml's `[tune]` section instead; validation still runs.
    self.validate()
}
