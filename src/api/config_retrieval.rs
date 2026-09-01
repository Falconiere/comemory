//! `api::config_retrieval` — `GET|PUT /api/v1/config/retrieval`: the live
//! ranking knobs with their ranges, and the validated partial update
//! (console-api spec §7).
//!
//! The update is validate-then-write, never write-then-validate: [`update`]
//! applies the supplied knobs to a CLONE of the live config, runs the same
//! `Config::validate` the file and env layers run, and only then patches
//! `config.toml` through the shared `config::patch::patch_config_file`
//! primitive. An out-of-range knob is therefore a `400` with the validator's
//! own message and leaves the file byte-identical (AC-14).
//!
//! Conn-free: neither function ever calls [`Ctx::conn`], so reading or
//! writing knobs on a data dir with no `comemory.db` does not create one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use toml::Value;

use crate::api::Ctx;
use crate::config::Config;
use crate::config::patch::{Table, patch_config_file, section};
use crate::prelude::*;

/// The declared bound of one knob, mirroring `config::validate`. `min`/`max`
/// are the numeric bounds; `note` carries what a number cannot (exclusivity,
/// pair-wise constraints, what `0` means).
#[derive(Serialize, Debug)]
pub struct Range {
    /// Lower bound, when the knob has one.
    pub min: Option<f64>,
    /// Upper bound, when the knob has one.
    pub max: Option<f64>,
    /// The part of the rule the two numbers cannot express.
    pub note: &'static str,
}

/// The live ranking knobs plus the per-knob [`Range`] table a console needs
/// to render inputs that cannot produce a `400`.
#[derive(Serialize, Debug)]
pub struct RetrievalKnobs {
    /// RRF fusion constant (`[retrieval] rrf_k`).
    pub rrf_k: f32,
    /// ACT-R decay exponent (`[rank] decay`).
    pub decay: f64,
    /// MMR relevance-vs-diversity lambda (`[rank] mmr_lambda`).
    pub mmr_lambda: f64,
    /// `memory_fts` BM25 column weights, `(body, tags)`.
    pub bm25_weights: (f32, f32),
    /// Graph-expansion hop depth; `0` disables the leg.
    pub graph_hops: u32,
    /// Provisional top hits seeding the graph-expansion walk.
    pub graph_seeds: usize,
    /// Minimum cosine similarity for memory ANN hits.
    pub memory_threshold: f32,
    /// Minimum cosine similarity for code ANN hits.
    pub code_threshold: f32,
    /// Results the hybrid router returns (also the default page size).
    pub top_k: usize,
    /// Weighted-RRF contribution of the document leg.
    pub document_leg_weight: f32,
    /// Rerank prior clamp `(lo, hi)` (`[rank] prior_clamp`).
    pub prior_clamp: (f64, f64),
    /// Per-knob bounds, keyed by the knob's `config.toml` name.
    pub ranges: BTreeMap<&'static str, Range>,
}

/// `PUT /api/v1/config/retrieval` request: every knob optional, and only
/// the supplied ones are written into `config.toml`.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateRequest {
    /// New `[retrieval] rrf_k`.
    #[serde(default)]
    pub rrf_k: Option<f32>,
    /// New `[rank] decay`.
    #[serde(default)]
    pub decay: Option<f64>,
    /// New `[rank] mmr_lambda`.
    #[serde(default)]
    pub mmr_lambda: Option<f64>,
    /// New `[retrieval] bm25_weights`, `(body, tags)`.
    #[serde(default)]
    pub bm25_weights: Option<(f32, f32)>,
    /// New `[retrieval] graph_hops`.
    #[serde(default)]
    pub graph_hops: Option<u32>,
    /// New `[retrieval] graph_seeds`.
    #[serde(default)]
    pub graph_seeds: Option<usize>,
    /// New `[retrieval] memory_threshold`.
    #[serde(default)]
    pub memory_threshold: Option<f32>,
    /// New `[retrieval] code_threshold`.
    #[serde(default)]
    pub code_threshold: Option<f32>,
    /// New `[retrieval] top_k`.
    #[serde(default)]
    pub top_k: Option<usize>,
    /// New `[retrieval] document_leg_weight`.
    #[serde(default)]
    pub document_leg_weight: Option<f32>,
    /// New `[rank] prior_clamp`, `(lo, hi)`.
    #[serde(default)]
    pub prior_clamp: Option<(f64, f64)>,
}

/// The declared bounds, one row per knob: `(name, min, max, note)`. A
/// const table rather than eleven struct literals — the shape is uniform
/// and the whole point is that it mirrors `config::validate` line for line.
const RANGES: &[(&str, Option<f64>, Option<f64>, &str)] = &[
    ("rrf_k", Some(0.0), None, "finite, strictly greater than 0"),
    ("decay", Some(0.0), None, "finite, >= 0"),
    ("mmr_lambda", Some(0.0), Some(1.0), "finite, in [0.0, 1.0]"),
    (
        "bm25_weights",
        Some(0.0),
        None,
        "two weights (body, tags); each finite and >= 0, at least one > 0",
    ),
    (
        "graph_hops",
        Some(0.0),
        Some(4.0),
        "integer; 0 disables the graph-expansion leg",
    ),
    ("graph_seeds", Some(1.0), None, "integer, >= 1"),
    (
        "memory_threshold",
        Some(0.0),
        Some(1.0),
        "cosine similarity floor, finite, in [0.0, 1.0]",
    ),
    (
        "code_threshold",
        Some(0.0),
        Some(1.0),
        "cosine similarity floor, finite, in [0.0, 1.0]",
    ),
    ("top_k", Some(1.0), None, "integer, >= 1"),
    (
        "document_leg_weight",
        Some(0.0),
        Some(10.0),
        "finite, in (0.0, 10.0] — 0 is rejected, use --only to drop the leg",
    ),
    (
        "prior_clamp",
        Some(0.0),
        None,
        "(lo, hi); both finite, lo > 0, lo <= hi",
    ),
];

/// Project a config onto the console's knob view.
pub fn get(cfg: &Config) -> RetrievalKnobs {
    RetrievalKnobs {
        rrf_k: cfg.retrieval.rrf_k,
        decay: cfg.rank.decay,
        mmr_lambda: cfg.rank.mmr_lambda,
        bm25_weights: cfg.retrieval.bm25_weights,
        graph_hops: cfg.retrieval.graph_hops,
        graph_seeds: cfg.retrieval.graph_seeds,
        memory_threshold: cfg.retrieval.memory_threshold,
        code_threshold: cfg.retrieval.code_threshold,
        top_k: cfg.retrieval.top_k,
        document_leg_weight: cfg.retrieval.document_leg_weight,
        prior_clamp: cfg.rank.prior_clamp,
        ranges: RANGES
            .iter()
            .map(|(name, min, max, note)| {
                (
                    *name,
                    Range {
                        min: *min,
                        max: *max,
                        note,
                    },
                )
            })
            .collect(),
    }
}

/// Validate the requested knobs against the live config, then write only
/// the supplied keys into `config.toml`. Returns the knobs as they now
/// stand (the validated in-memory config's — the file and it agree, since
/// the write is the last fallible step and any failure propagates).
pub fn update(ctx: &mut Ctx<'_>, req: UpdateRequest) -> Result<RetrievalKnobs> {
    let mut candidate = ctx.cfg.clone();
    overlay(&mut candidate, &req);
    // The validator's message names the field AND its env var, which is
    // exactly what a `400` body should say; only its class changes.
    let validated = candidate
        .validate()
        .map_err(|e| Error::BadRequest(e.to_string()))?;
    patch_config_file(&ctx.paths.config_file(), |table| write_knobs(table, &req))?;
    Ok(get(&validated))
}

/// Apply every supplied knob to `cfg`, leaving the rest untouched.
fn overlay(cfg: &mut Config, req: &UpdateRequest) {
    if let Some(v) = req.rrf_k {
        cfg.retrieval.rrf_k = v;
    }
    if let Some(v) = req.decay {
        cfg.rank.decay = v;
    }
    if let Some(v) = req.mmr_lambda {
        cfg.rank.mmr_lambda = v;
    }
    if let Some(v) = req.bm25_weights {
        cfg.retrieval.bm25_weights = v;
    }
    if let Some(v) = req.graph_hops {
        cfg.retrieval.graph_hops = v;
    }
    if let Some(v) = req.graph_seeds {
        cfg.retrieval.graph_seeds = v;
    }
    if let Some(v) = req.memory_threshold {
        cfg.retrieval.memory_threshold = v;
    }
    if let Some(v) = req.code_threshold {
        cfg.retrieval.code_threshold = v;
    }
    if let Some(v) = req.top_k {
        cfg.retrieval.top_k = v;
    }
    if let Some(v) = req.document_leg_weight {
        cfg.retrieval.document_leg_weight = v;
    }
    if let Some(v) = req.prior_clamp {
        cfg.rank.prior_clamp = v;
    }
}

/// A `(f32, f32)` pair as a TOML array of floats — the encoding
/// `eval::tune::write_candidate` already uses for `bm25_weights`, kept
/// identical so the two writers produce the same file shape.
fn pair(a: f64, b: f64) -> Value {
    Value::Array(vec![Value::Float(a), Value::Float(b)])
}

/// Write only the supplied knobs into `[retrieval]` / `[rank]`. Absent
/// fields are never written, so an update cannot materialize a key the
/// operator never set.
fn write_knobs(table: &mut Table, req: &UpdateRequest) -> Result<()> {
    {
        let r = section(table, "retrieval")?;
        if let Some(v) = req.rrf_k {
            r.insert("rrf_k".into(), Value::Float(f64::from(v)));
        }
        if let Some(v) = req.bm25_weights {
            r.insert("bm25_weights".into(), pair(f64::from(v.0), f64::from(v.1)));
        }
        if let Some(v) = req.graph_hops {
            r.insert("graph_hops".into(), Value::Integer(i64::from(v)));
        }
        if let Some(v) = req.graph_seeds {
            r.insert("graph_seeds".into(), Value::Integer(v as i64));
        }
        if let Some(v) = req.memory_threshold {
            r.insert("memory_threshold".into(), Value::Float(f64::from(v)));
        }
        if let Some(v) = req.code_threshold {
            r.insert("code_threshold".into(), Value::Float(f64::from(v)));
        }
        if let Some(v) = req.top_k {
            r.insert("top_k".into(), Value::Integer(v as i64));
        }
        if let Some(v) = req.document_leg_weight {
            r.insert("document_leg_weight".into(), Value::Float(f64::from(v)));
        }
    }
    let rank = section(table, "rank")?;
    if let Some(v) = req.decay {
        rank.insert("decay".into(), Value::Float(v));
    }
    if let Some(v) = req.mmr_lambda {
        rank.insert("mmr_lambda".into(), Value::Float(v));
    }
    if let Some(v) = req.prior_clamp {
        rank.insert("prior_clamp".into(), pair(v.0, v.1));
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/config_retrieval.rs"]
mod tests;
