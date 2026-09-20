//! The console search response's shape: one finished `find` run turned into
//! the `data` object `GET|POST /api/v1/search` answers with.
//!
//! Its own file beside `search.rs` so that file stays under the 300-line
//! ceiling (Binding Rule 3). No `serve/routes` subfolder: guardrails allow
//! only `memories`, `maint` and `tests` there.

use serde::Serialize;

use crate::domains::retrieval;
use crate::domains::retrieval::explain::{self, ExplainPart};
use crate::domains::retrieval::unified::fuse_domains::UnifiedHit;
use crate::serve::routes::search::TIER_COUNT;

/// One hit as the console reads it: the unified hit's fields plus `type`
/// (an alias of `domain`, which is what the draft spec's clients key on)
/// and the derived explain strip.
#[derive(Serialize)]
struct ConsoleHit {
    /// Memory id, `code_symbols` id, or document id.
    id: String,
    /// Domain label under the draft spec's field name.
    #[serde(rename = "type")]
    hit_type: String,
    /// Domain label under the pipeline's own field name.
    domain: String,
    /// Human-readable headline.
    title: String,
    /// The dim second line.
    subtitle: String,
    /// Owning repo, where the domain has one.
    repo: Option<String>,
    /// File path, where the domain has one.
    path: Option<String>,
    /// Fused score.
    score: f64,
    /// 1-based position within this hit's own domain.
    rank_in_domain: usize,
    /// The derived explain strip; omitted entirely when `explain: false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    score_parts: Option<Vec<ExplainPart>>,
}

/// Shape one finished run into the console's response `data`.
pub(super) fn body(
    out: &retrieval::find::FindResult,
    explain_hits: bool,
    took_ms: u64,
    rrf_k: f32,
) -> serde_json::Value {
    let hits: Vec<ConsoleHit> = out
        .hits
        .iter()
        .map(|h| console_hit(h, explain_hits))
        .collect();
    serde_json::json!({
        "query_id": out.query_id,
        "took_ms": took_ms,
        "fusion": { "method": "rrf", "k": f64::from(rrf_k) },
        // The DEEPEST ladder tier any memory hit needed. A code or document
        // hit carries no tier (it never ran the memory ladder), so this is
        // `null` on a code-only page rather than a misleading `1`.
        "tier": out.hits.iter().filter_map(|h| h.tier).max(),
        "tier_count": TIER_COUNT,
        "hits": hits,
        "limit": out.meta.limit,
        "offset": out.meta.offset,
        "has_more": out.meta.has_more,
        "total": out.meta.total,
    })
}

/// Project one [`UnifiedHit`], deriving its explain strip when asked.
fn console_hit(h: &UnifiedHit, explain_hits: bool) -> ConsoleHit {
    ConsoleHit {
        id: h.id.clone(),
        hit_type: h.domain.clone(),
        domain: h.domain.clone(),
        title: h.title.clone(),
        subtitle: h.subtitle.clone(),
        repo: h.repo.clone(),
        path: h.path.clone(),
        score: h.score,
        rank_in_domain: h.rank_in_domain,
        score_parts: explain_hits.then(|| explain::parts_of(&h.score_parts)),
    }
}
