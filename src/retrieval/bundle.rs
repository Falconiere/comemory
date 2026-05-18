//! Wire-format types returned by the retrieval pipeline. No behaviour lives
//! here — these are pure serializable shapes the CLI/MCP layer renders.

use serde::Serialize;

/// One citation-ready hit inside a [`Bundle`]. `score` is the post-rank
/// numeric the caller may render; `why` is a short human-readable reason
/// (e.g. `"vector top-1, gap 0.32"`) attached by the pipeline.
#[derive(Debug, Clone, Serialize)]
pub struct CitedHit {
    pub id: String,
    pub score: f32,
    pub kind: String,
    pub repo: String,
    pub snippet: String,
    pub why: String,
}

/// Full retrieval result for a query: the route the router picked, the
/// surviving hits, the confidence gap between top-1 and top-2, and whether
/// the corrective fallback fired. Designed for direct serde_json emission
/// behind `--json`.
#[derive(Debug, Clone, Serialize)]
pub struct Bundle {
    pub query: String,
    pub route: String,
    pub hits: Vec<CitedHit>,
    pub confidence: f32,
    pub fallback_used: bool,
}
