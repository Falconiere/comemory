//! `GET /api/search?q=<phrase>&k=<n>` — ranked FILE hits for the web viewer.
//!
//! Runs the shared [`crate::retrieval::code_search::search_code_hits`] core and
//! coalesces the per-symbol rows into per-file hits keyed by the same
//! `file:<repo>:<path>` id `GET /api/graph` emits. Lexical by default; hybrid
//! when an embed command is configured. An embed-command failure — or any
//! vectored-retrieval failure (e.g. a wrong-dim embedding) — DEGRADES to
//! lexical (logged, not surfaced); an empty query or empty index returns `200`
//! with `hits: []`.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};

use crate::graph::edges::file_node_id;
use crate::retrieval::code_search::search_code_hits;
use crate::retrieval::pipeline;
use crate::serve::AppState;
use crate::serve::error::ApiError;

/// One ranked file hit, keyed by the same `file:<repo>:<path>` id the graph
/// endpoint emits so the frontend can map it 1:1 onto a graph node.
#[derive(Serialize)]
pub struct FileHit {
    /// `file:<repo>:<path>` — matches a `GET /api/graph` node id exactly.
    pub node_id: String,
    /// Repository the file was indexed from.
    pub repo: String,
    /// Repo-relative file path.
    pub path: String,
    /// Best `final_score` among the file's matched symbols (descending sort key).
    pub score: f64,
    /// Qualified name of the highest-scoring symbol in the file.
    pub top_symbol: String,
}

/// Envelope returned by `GET /api/search`.
#[derive(Serialize)]
pub struct SearchResult {
    /// The query string as received (echoed for the client).
    pub query: String,
    /// `"hybrid"` when an embed vector was used, else `"lexical"` (the
    /// degrade path also reports `"lexical"`).
    pub mode: &'static str,
    /// Ranked file hits, highest `score` first, at most `k` entries.
    pub hits: Vec<FileHit>,
}

/// `?q=<phrase>&k=<n>` query for the search endpoint.
#[derive(Deserialize)]
pub struct SearchQuery {
    /// Natural-language or identifier query.
    q: String,
    /// Page size; defaults to the configured `retrieval.top_k` when omitted.
    k: Option<usize>,
}

/// `GET /api/search` — ranked file hits for the query.
///
/// Empty/whitespace `q` short-circuits to `200` with no hits. When the
/// session carries an embed command the query is vectorized for a hybrid
/// search; if embedding fails the request degrades to lexical (logged, not
/// surfaced as an error). The reranked symbol hits are coalesced to one entry
/// per file (max `final_score` wins, that row's symbol becomes `top_symbol`),
/// sorted by score descending, and cut to `k`.
pub async fn search(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> std::result::Result<Json<SearchResult>, ApiError> {
    let query = q.q.trim().to_string();
    let cfg = state.cfg();
    // `k` of 0 (or absent) means "use the configured page size", not "return
    // nothing": `pool_size(_, 0, _)` reads 0 as the all-within-window sentinel,
    // so a literal `?k=0` would run a full search and then `truncate(0)` would
    // silently drop every hit. Treat any non-positive `k` as unset.
    let k = q.k.filter(|&k| k > 0).unwrap_or(cfg.retrieval.top_k);
    if query.is_empty() {
        return Ok(Json(SearchResult {
            query,
            mode: "lexical",
            hits: Vec::new(),
        }));
    }

    let (vector, mut mode) = resolve_vector(&state, &query);
    let pool = pipeline::pool_size(0, k, cfg.retrieval.max_page_window);

    let conn = state.conn()?;
    let ranked = match search_code_hits(
        cfg,
        &conn,
        &query,
        vector.as_deref(),
        state.repo(),
        None,
        pool,
    ) {
        Ok(r) => r,
        // A "successful" embed can still yield a wrong-dimension vector (a
        // mismatched embedder), which the vec0 dim guard rejects with
        // `VecDimMismatch`. That is the same DEGRADE intent as an embed-cmd
        // failure: log, drop the vector, retry lexical. Scope the retry to that
        // one fault so a genuine DB error (I/O, corruption) still surfaces as a
        // 5xx instead of masquerading as an empty lexical result.
        Err(e @ crate::errors::Error::VecDimMismatch { .. }) if vector.is_some() => {
            tracing::warn!(error = %e, "serve search: embedding dim mismatch; retrying lexical");
            mode = "lexical";
            search_code_hits(cfg, &conn, &query, None, state.repo(), None, pool)?
        }
        Err(e) => return Err(e.into()),
    };

    let hits = coalesce_to_files(ranked, k);
    Ok(Json(SearchResult { query, mode, hits }))
}

/// Resolve the query vector and the result mode. With no embed command the
/// search is lexical (`None`). With one configured, embedding success yields a
/// vector + `"hybrid"`; failure is the DEGRADE path — log via `tracing::warn!`
/// and fall back to lexical (`None`, `"lexical"`) so the request still succeeds.
fn resolve_vector(state: &AppState, query: &str) -> (Option<Vec<f32>>, &'static str) {
    match state.embed_cmd() {
        None => (None, "lexical"),
        Some(cmd) => match crate::embed::embed_query(cmd, query) {
            Ok(v) => (Some(v), "hybrid"),
            Err(e) => {
                tracing::warn!(error = %e, "serve search: embed-cmd failed; falling back to lexical");
                (None, "lexical")
            }
        },
    }
}

/// Fold the reranked symbol hits into per-file hits: group by
/// `file_node_id(repo, path)`, keep the max `parts.final_score` per file (that
/// row's symbol becomes `top_symbol`), sort by score descending (ties on
/// `node_id` for determinism), and take `k`.
fn coalesce_to_files(
    ranked: Vec<crate::retrieval::code_rerank::CodeReranked>,
    k: usize,
) -> Vec<FileHit> {
    let mut best: HashMap<String, FileHit> = HashMap::new();
    for hit in ranked {
        let node_id = file_node_id(&hit.repo, &hit.path);
        let score = hit.parts.final_score;
        match best.get_mut(&node_id) {
            Some(existing) if existing.score >= score => {}
            Some(existing) => {
                existing.score = score;
                existing.top_symbol = hit.symbol;
            }
            None => {
                best.insert(
                    node_id.clone(),
                    FileHit {
                        node_id,
                        repo: hit.repo,
                        path: hit.path,
                        score,
                        top_symbol: hit.symbol,
                    },
                );
            }
        }
    }
    let mut hits: Vec<FileHit> = best.into_values().collect();
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.node_id.cmp(&b.node_id))
    });
    hits.truncate(k);
    hits
}
