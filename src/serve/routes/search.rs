//! `GET|POST /api/v1/search`, `GET /api/v1/search/suggest`,
//! `POST /api/v1/search/{query_id}/feedback` (console-api spec §3).
//!
//! `/search` is a **transport adapter, not a second ranking path** (spec
//! Non-Goal 9): it renames the console's field names onto
//! `api::find::Request`, runs the one unified pipeline, and reshapes the
//! answer. Nothing is re-scored here — the explain strip is derived from
//! each hit's existing `score_parts` by [`crate::output::explain`], and
//! `fusion`/`tier` merely report what the pipeline already did.
//!
//! Its `RouteEntry` command is the dotted synthetic name `search.console`
//! rather than `find`: the two paths share a core but not a request shape,
//! and `GET /commands` should not claim otherwise.

use std::time::Instant;

use axum::Router;
use axum::extract::{Json, Path, Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use serde::{Deserialize, Deserializer, Serialize};

use crate::api::{self, Ctx};
use crate::memory::Kind;
use crate::output::explain::{self, ExplainPart};
use crate::prelude::*;
use crate::retrieval::unified::fuse_domains::UnifiedHit;
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, guard_mutating, respond, run_blocking, track_for};
use crate::serve::scope::RepoScope;

/// How many lexical ladder tiers the memory router has (strict, word-OR,
/// subtoken-OR, learned expansion) — echoed so a console can render
/// `tier / tier_count` without hardcoding the ladder's depth.
const TIER_COUNT: u8 = 4;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/search",
            command: "search.console",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/search",
            command: "search.console",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/search/suggest",
            command: "search.suggest",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/search/{query_id}/feedback",
            command: "search.feedback",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/search", get(search_get).post(search_post))
        .route("/api/v1/search/suggest", get(suggest))
        .route("/api/v1/search/{query_id}/feedback", post(feedback))
}

/// The console's own search request shape. Adapted onto
/// [`api::find::Request`] by [`into_find`]; never reaches the pipeline as-is.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct ConsoleSearch {
    /// The query text. `query` is accepted as an alias so a caller written
    /// against `/find` can post the same body here.
    #[serde(alias = "query")]
    q: String,
    /// `all` (default) | `memories` | `code` | `documents`. The singular
    /// spellings the pipeline itself uses (`memory`, `document`) are
    /// accepted too.
    #[serde(default)]
    scope: Option<String>,
    /// Repo filter, defaulted from `X-Comemory-Repo` when omitted.
    #[serde(default)]
    repo: Option<String>,
    /// Memory-kind filter. At most one (spec Non-Goal 5) — the pipeline's
    /// `Filters.kind` is a single value, and pretending otherwise would
    /// silently drop the rest. A JSON array on `POST`, or a comma-separated
    /// string on either form (`GET ?kinds=bug` — see [`kinds_field`]).
    #[serde(default, deserialize_with = "kinds_field")]
    kinds: Vec<String>,
    /// Page size (`api::find`'s `k`).
    #[serde(default)]
    limit: Option<usize>,
    /// Ranked results to skip.
    #[serde(default)]
    offset: usize,
    /// Whether each hit carries its derived `score_parts` strip. Default
    /// `true`: the console's result list shows it inline.
    #[serde(default = "explain_default")]
    explain: bool,
    /// Caller-supplied dense vector. `POST` only — an embedding does not
    /// fit in a query string.
    #[serde(default)]
    vector: Option<Vec<f32>>,
}

/// `explain` defaults to on.
fn explain_default() -> bool {
    true
}

/// `kinds` in either spelling a client can send: a JSON sequence
/// (`{"kinds":["bug"]}`) or a comma-separated string (`?kinds=bug,decision`).
/// A query string has no sequence syntax `serde_urlencoded` can decode, so
/// without the string form every `GET ?kinds=` was a plain-text `400` while
/// the docs promised a `GET|POST` filter. Blank entries are dropped, so
/// `?kinds=` reads as no filter.
fn kinds_field<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Kinds {
        List(Vec<String>),
        Csv(String),
    }
    Ok(match Kinds::deserialize(d)? {
        Kinds::List(list) => list,
        Kinds::Csv(csv) => csv
            .split(',')
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .collect(),
    })
}

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

/// `GET /api/v1/search` — query-string form, no vector.
async fn search_get(
    State(state): State<AppState>,
    scope: RepoScope,
    Query(mut req): Query<ConsoleSearch>,
) -> Response {
    req.repo = scope.resolve(req.repo);
    execute(state, req).await
}

/// `POST /api/v1/search` — body form, vector-capable.
async fn search_post(
    State(state): State<AppState>,
    scope: RepoScope,
    Json(mut req): Json<ConsoleSearch>,
) -> Response {
    req.repo = scope.resolve(req.repo);
    execute(state, req).await
}

/// Shared handler body: adapt, run, reshape. Access tracking is suppressed
/// on a read-only server exactly as it is for `find`.
async fn execute(state: AppState, req: ConsoleSearch) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let explain_hits = req.explain;
        let find = into_find(req)?;
        let track = track_for(&state)?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        let run_started = Instant::now();
        let out = api::find::run(&mut ctx, find, track)?;
        let took_ms = u64::try_from(run_started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(body(&out, explain_hits, took_ms, cfg.retrieval.rrf_k))
    })
    .await;
    respond("search.console", result, started)
}

/// Shape one finished run into the console's response `data`.
fn body(
    out: &api::find::FindResult,
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

/// Adapt the console's request onto the pipeline's. The time-scoping,
/// language, and document-path filters have no console control yet and are
/// left at their defaults rather than invented here.
fn into_find(req: ConsoleSearch) -> Result<api::find::Request> {
    Ok(api::find::Request {
        query: req.q,
        k: req.limit,
        offset: req.offset,
        domain: Some(domain_of(req.scope.as_deref())?.to_string()),
        repo: req.repo,
        kind: kind_of(&req.kinds)?,
        lang: None,
        path: Vec::new(),
        vector: req.vector,
        since: None,
        until: None,
        as_of: None,
    })
}

/// `scope` → `api::find`'s `domain`. Both the console's plural spellings
/// and the pipeline's own singular ones are accepted; anything else names
/// the offender rather than silently searching everything.
fn domain_of(scope: Option<&str>) -> Result<&'static str> {
    match scope.unwrap_or("all") {
        "all" => Ok("all"),
        "memories" | "memory" => Ok("memory"),
        "code" => Ok("code"),
        "documents" | "document" => Ok("document"),
        other => Err(Error::BadRequest(format!(
            "unknown scope `{other}`: expected all, memories, code, or documents"
        ))),
    }
}

/// `kinds[]` → the pipeline's single `kind` (spec Non-Goal 5).
fn kind_of(kinds: &[String]) -> Result<Option<Kind>> {
    match kinds {
        [] => Ok(None),
        [one] => {
            let value = serde_json::Value::String(one.to_ascii_lowercase());
            serde_json::from_value::<Kind>(value)
                .map(Some)
                .map_err(|_| Error::BadRequest(format!("unknown kind `{one}`")))
        }
        _ => Err(Error::BadRequest("one kind per query".into())),
    }
}

/// `GET /api/v1/search/suggest` — mined expansions + recent queries.
async fn suggest(
    State(state): State<AppState>,
    Query(req): Query<api::suggest::Request>,
) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::suggest::run(&mut ctx, req)
    })
    .await;
    respond("search.suggest", result, started)
}

/// Per-hit feedback as the console sends it, adapted onto
/// [`api::feedback::Request`]'s four id lists by [`into_feedback`].
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct HitFeedback {
    /// The hit's id: a memory id, or a `code_symbols` id as a string.
    hit_id: String,
    /// `memory` (default) | `code` — which of the two id spaces `hit_id`
    /// belongs to.
    #[serde(rename = "type", default)]
    hit_type: Option<String>,
    /// `used` | `opened` | `ignored`.
    signal: String,
    /// `explicit` | `implicit`. Accepted and validated-by-shape only:
    /// `feedback_events` records provenance per verdict, not per source,
    /// so there is nowhere to store this yet. Documented as informational
    /// rather than dropped from the schema, so a console can send it today.
    #[serde(default)]
    source: Option<String>,
}

/// `POST /api/v1/search/{query_id}/feedback` — record one hit's verdict.
/// Mutating: [`guard_mutating`] runs first and its permit is held inside
/// the blocking closure, exactly as `POST /feedback` does.
async fn feedback(
    State(state): State<AppState>,
    Path(query_id): Path<String>,
    Json(req): Json<HitFeedback>,
) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("search.feedback", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let request = into_feedback(query_id, req)?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::feedback::run(&mut ctx, request)
    })
    .await;
    respond("search.feedback", result, started)
}

/// Adapt one hit verdict onto the four-list request. `used` and `opened`
/// both count as used — opening a hit is the weaker signal, but the
/// aggregated `feedback` table has one positive counter, and inventing a
/// third verdict here would change the ranking contract.
fn into_feedback(query_id: String, req: HitFeedback) -> Result<api::feedback::Request> {
    let positive = match req.signal.as_str() {
        "used" | "opened" => true,
        "ignored" => false,
        other => {
            return Err(Error::BadRequest(format!(
                "unknown signal `{other}`: expected used, opened, or ignored"
            )));
        }
    };
    if let Some(source) = req.source.as_deref()
        && !matches!(source, "explicit" | "implicit")
    {
        return Err(Error::BadRequest(format!(
            "unknown source `{source}`: expected explicit or implicit"
        )));
    }
    let mut out = api::feedback::Request {
        query_id,
        used: Vec::new(),
        irrelevant: Vec::new(),
        used_code: Vec::new(),
        irrelevant_code: Vec::new(),
    };
    match (req.hit_type.as_deref().unwrap_or("memory"), positive) {
        ("memory", true) => out.used.push(req.hit_id),
        ("memory", false) => out.irrelevant.push(req.hit_id),
        ("code", true) => out.used_code.push(req.hit_id),
        ("code", false) => out.irrelevant_code.push(req.hit_id),
        (other, _) => {
            return Err(Error::BadRequest(format!(
                "unknown type `{other}`: expected memory or code"
            )));
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests/search.rs"]
mod tests;
