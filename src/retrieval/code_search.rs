//! Shared code-retrieval orchestration: route candidates, build the
//! working set, and rerank — the route → rerank core that both
//! `comemory search-code` (`crate::cli::search_code`) and any future
//! programmatic caller (e.g. an HTTP `/api/search` handler) run.
//!
//! Pagination, telemetry, and rendering stay with the caller: this helper
//! returns the full ranked (post-coalesce) window so each caller can
//! slice/coalesce it on its own terms.

use rusqlite::Connection;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::code_rerank::{self, CodeReranked, WorkingSet};
use crate::retrieval::code_route;

/// Route + rerank a code query into the full ranked window of
/// [`CodeReranked`] hits, the shared core of `comemory search-code`.
///
/// Routes candidates via [`code_route::route_code`] (BM25 + optional
/// thresholded ANN, RRF-fused), builds the working set for the affinity
/// prior — skipping the git discovery + status walk behind
/// [`WorkingSet::from_cwd`] entirely when routing found nothing, since a
/// zero-candidate pool has nothing to boost — and reranks via
/// [`code_rerank::rerank_code`]. The returned Vec is the full ranked
/// (post-coalesce) window; callers paginate / coalesce it themselves.
pub fn search_code_hits(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    vec: Option<&[f32]>,
    repo: Option<&str>,
    lang: Option<&str>,
    pool: usize,
) -> Result<Vec<CodeReranked>> {
    let candidates = code_route::route_code(cfg, conn, query, vec, repo, lang, pool)?;
    let ws = if candidates.is_empty() {
        WorkingSet::default()
    } else {
        WorkingSet::from_cwd(repo)
    };
    code_rerank::rerank_code(conn, cfg, &candidates, &ws)
}
