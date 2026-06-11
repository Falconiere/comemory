//! `COMEMORY_*` env-var overrides for [`Config`].
//!
//! Split out of `file.rs` to keep each config file narrow: `file.rs` owns
//! the struct definitions, defaults, file overlay, and shared validation;
//! this module owns env parsing. Both entry points funnel through the same
//! `Config::validate` pass so file and env layers enforce identical
//! invariants.

use super::file::{AutoReindexMode, Config};
use crate::prelude::*;

/// Read an env var as `f64`; `Ok(None)` when unset, `Err` on parse failure.
fn env_f64(name: &str) -> Result<Option<f64>> {
    let Ok(v) = std::env::var(name) else {
        return Ok(None);
    };
    let parsed = v
        .parse::<f64>()
        .map_err(|e| Error::Other(format!("invalid env var {name}={v}: {e}")))?;
    Ok(Some(parsed))
}

/// Read an env var as `u32`; `Ok(None)` when unset, `Err` on parse failure.
fn env_u32(name: &str) -> Result<Option<u32>> {
    let Ok(v) = std::env::var(name) else {
        return Ok(None);
    };
    let parsed = v
        .parse::<u32>()
        .map_err(|e| Error::Other(format!("invalid env var {name}={v}: {e}")))?;
    Ok(Some(parsed))
}

/// Read an env var as an `"a,b"` pair of numbers; `Ok(None)` when unset.
///
/// Only the shape (exactly two comma-separated values that parse as `T`)
/// is checked here; the range invariants (finite, ordering, sign) live in
/// `Config::validate` so the file overlay enforces them identically.
/// Shared by `COMEMORY_RANK_PRIOR_CLAMP` (`f64`) and
/// `COMEMORY_RETRIEVAL_BM25_WEIGHTS` (`f32`).
fn env_pair<T: std::str::FromStr>(name: &str) -> Result<Option<(T, T)>>
where
    T::Err: std::fmt::Display,
{
    let Ok(v) = std::env::var(name) else {
        return Ok(None);
    };
    let parts: Vec<&str> = v.split(',').collect();
    if parts.len() != 2 {
        return Err(Error::Other(format!(
            "invalid env var {name}={v}: expected \"a,b\" (two comma-separated numbers)"
        )));
    }
    let a = parts[0]
        .trim()
        .parse::<T>()
        .map_err(|e| Error::Other(format!("invalid env var {name}={v}: first value: {e}")))?;
    let b = parts[1]
        .trim()
        .parse::<T>()
        .map_err(|e| Error::Other(format!("invalid env var {name}={v}: second value: {e}")))?;
    Ok(Some((a, b)))
}

impl Config {
    /// Apply `COMEMORY_*` env-var overrides on top of `self`.
    ///
    /// Unlike the previous infallible variant, parse failures (non-numeric
    /// `top_k` / thresholds, unknown `auto_reindex` mode, unknown boolean for
    /// `auto_sync`) are now surfaced as `Error::Other` rather than silently
    /// dropped. This catches typos at startup instead of letting them mask as
    /// "defaults applied". Rank/prune range invariants are enforced by the
    /// shared `Config::validate` pass at the end.
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
                Error::Other(format!("invalid env var COMEMORY_RETRIEVAL_TOP_K={v}: {e}"))
            })?;
        }
        if let Ok(v) = std::env::var("COMEMORY_RETRIEVAL_MEMORY_THRESHOLD") {
            self.retrieval.memory_threshold = v.parse::<f32>().map_err(|e| {
                Error::Other(format!(
                    "invalid env var COMEMORY_RETRIEVAL_MEMORY_THRESHOLD={v}: {e}"
                ))
            })?;
        }
        // Only the parse happens here; the finite/positive invariant lives
        // in `Config::validate` so the file overlay is checked identically.
        if let Ok(v) = std::env::var("COMEMORY_RETRIEVAL_RRF_K") {
            self.retrieval.rrf_k = v.parse::<f32>().map_err(|e| {
                Error::Other(format!("invalid env var COMEMORY_RETRIEVAL_RRF_K={v}: {e}"))
            })?;
        }
        if let Some(v) = env_pair::<f32>("COMEMORY_RETRIEVAL_BM25_WEIGHTS")? {
            self.retrieval.bm25_weights = v;
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
        if let Some(v) = env_f64("COMEMORY_RANK_DECAY")? {
            self.rank.decay = v;
        }
        if let Some(v) = env_pair::<f64>("COMEMORY_RANK_PRIOR_CLAMP")? {
            self.rank.prior_clamp = v;
        }
        if let Some(v) = env_f64("COMEMORY_RANK_MMR_LAMBDA")? {
            self.rank.mmr_lambda = v;
        }
        if let Some(v) = env_f64("COMEMORY_PRUNE_MIN_ACTIVATION")? {
            self.prune.min_activation = v;
        }
        if let Some(v) = env_f64("COMEMORY_PRUNE_MIN_FEEDBACK")? {
            self.prune.min_feedback = v;
        }
        if let Some(v) = env_u32("COMEMORY_PRUNE_BELOW_QUALITY")? {
            self.prune.low_value_default_below_quality = v;
        }
        self.validate()
    }
}
