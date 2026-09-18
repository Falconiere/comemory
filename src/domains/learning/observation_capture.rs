//! Opt-in, bounded capture of a real query's candidate pool (#209).
//!
//! Turns one `comemory find` run into the #208 candidate observation contract
//! and persists it. The passage is read by `evaluation::candidate_facts` while
//! each candidate's identity is still known — before `fuse_domains` flattens
//! the legs and drops it — so nothing here ever reconstructs text from the
//! corpus as it stands later.
//!
//! Capture is best effort end to end: [`record`] swallows every failure into a
//! `tracing::warn!`, exactly as `retrieval::pipeline::log_retrieval` does, so a
//! search stays usable when the optional capture cannot write.

use time::OffsetDateTime;

use crate::config::Config;
use crate::domains::learning::evaluation::benchmark_observe::{
    FilterInputs, effective_filters, observe,
};
use crate::domains::learning::evaluation::candidate_facts::FactsByHit;
use crate::domains::learning::evaluation::candidate_observation::{
    CandidateObservation, EffectiveFilters, OBSERVATION_VERSION, VectorScenario,
};
use crate::domains::learning::evaluation::run_environment::retrieval_version;
use crate::domains::retrieval::scope::Filters;
use crate::domains::retrieval::unified::{DomainFilters, fuse_domains};
use crate::prelude::*;
use crate::store::candidate_observations::{NewCandidate, NewObservation, insert};
use crate::store::{Connection, memory_row};
use crate::utilities::dated_id::dated_id;
use crate::utilities::pagination::PageWindow;

/// Id prefix of a captured observation, in the shared
/// `<prefix>-<yyyymmdd>-<8hex>` shape retrieval query ids use.
pub const OBSERVATION_ID_PREFIX: &str = "o";

/// Everything one captured run supplies that retrieval does not already hold.
pub struct CaptureInput<'a> {
    /// The query text, verbatim as retrieval received it.
    pub query: &'a str,
    /// The `retrieval_log.query_id` of the same run, when one was written.
    pub query_id: Option<&'a str>,
    /// Which surface produced the run (a `utilities::telemetry::source` const).
    pub source: &'a str,
    /// Every filter that was in effect, per leg.
    pub filters: EffectiveFilters,
    /// The window the page was sliced at.
    pub window: PageWindow,
    /// The shared pool size every leg was fetched at.
    pub pool_size: usize,
    /// The fused ranking, before pagination cut it.
    pub pool: &'a [fuse_domains::UnifiedHit],
    /// Identity, content version and bounded text, read before fusion.
    pub facts: &'a FactsByHit,
}

/// Whether this run may capture: the operator opted in, and the run is one
/// that may write telemetry at all.
///
/// Reusing `track` is what keeps a read-only `comemory serve` from writing:
/// `serve::routes::track_for` already returns `false` there, and `/find` keeps
/// its `mutating: false` route-table entry honestly without a second gate to
/// hold in sync.
pub fn armed(cfg: &Config, track: bool) -> bool {
    cfg.observations.enabled && track
}

/// The `EffectiveFilters` one `find` run applied. The benchmark side of the
/// same mapping is `benchmark_runner::task_filters`; both meet in
/// [`effective_filters`] so the record has one implementation.
pub fn find_filters(
    cfg: &Config,
    filters: Filters<'_>,
    domain_filters: DomainFilters<'_>,
    vector: Option<&[f32]>,
) -> EffectiveFilters {
    let inputs = FilterInputs {
        domains: filters.domains,
        repo: filters.repo,
        kind: filters.kind,
        lang: domain_filters.lang,
        path_globs: domain_filters.path_globs,
    };
    let scenario = match vector {
        Some(v) => VectorScenario::supplied(&embed_model(cfg), v),
        None => VectorScenario::Lexical,
    };
    effective_filters(inputs, filters.scope, scenario)
}

/// The embedder identity recorded alongside a caller-supplied vector.
///
/// comemory never embeds, so the only identity available is what the operator
/// declared: `[embed] model` first, then the free-form `embed_hint`. `unknown`
/// when neither is set — an honest placeholder, since a replay cannot claim
/// which embedder produced a vector nobody named.
fn embed_model(cfg: &Config) -> String {
    let declared = [Some(cfg.embed.model.as_str()), cfg.embed_hint.as_deref()];
    declared
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|v| !v.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

/// Build the observation and persist it, returning its new id.
///
/// Every error is the caller's to swallow; [`record`] is the entry point a
/// search uses.
pub fn capture(cfg: &Config, conn: &Connection, input: CaptureInput<'_>) -> Result<String> {
    let now = OffsetDateTime::now_utc();
    let at = memory_row::iso_format(now)?;
    let observation_id = dated_id(OBSERVATION_ID_PREFIX, input.query, now);
    let version = retrieval_version(cfg, conn)?;
    let (candidates, _unavailable) = observe(input.pool, input.facts, input.window);
    let cut = candidate_cut(cfg, input.window, candidates.len());
    let kept = &candidates[..cut];
    // The locator JSON is built first so each row can borrow it: a candidate
    // row holds `&str`, and serializing inside the map would drop the String
    // at the end of the closure.
    let locators = kept
        .iter()
        .map(|c| serde_json::to_string(&c.locator))
        .collect::<std::result::Result<Vec<String>, _>>()
        .map_err(Error::Json)?;
    let rows: Vec<NewCandidate<'_>> = kept
        .iter()
        .zip(locators.iter())
        .map(|(c, locator)| candidate_row(c, locator))
        .collect();
    let filters_json = serde_json::to_string(&input.filters).map_err(Error::Json)?;
    let retrieval_json = serde_json::to_string(&version).map_err(Error::Json)?;
    insert(
        conn,
        &NewObservation {
            observation_id: &observation_id,
            observation_version: i64::from(OBSERVATION_VERSION),
            query_id: input.query_id,
            query: input.query,
            source: input.source,
            filters_json: &filters_json,
            retrieval_json: &retrieval_json,
            knobs_hash: &version.knobs_hash,
            corpus_digest: &version.corpus.digest,
            decay_frozen: version.knobs.decay_frozen(),
            pool_size: as_i64(input.pool_size),
            page_limit: as_i64(input.window.limit),
            page_offset: as_i64(input.window.offset),
            truncated: cut < candidates.len(),
            at: &at,
        },
        &rows,
    )?;
    Ok(observation_id)
}

/// [`capture`], best effort: a failure is warned and reported as "no
/// observation", never raised. A search must stay usable when an optional
/// capture cannot write — the whole point of capture being opt-in.
pub fn record(cfg: &Config, conn: &Connection, input: CaptureInput<'_>) -> Option<String> {
    match capture(cfg, conn, input) {
        Ok(id) => Some(id),
        Err(e) => {
            tracing::warn!(error = %e, "candidate observation capture failed; search unaffected");
            None
        }
    }
}

/// How many pooled candidates to persist: the configured ceiling, raised when
/// it would otherwise cut into the page the caller was handed.
///
/// A returned candidate must never be dropped — a judgment addresses what the
/// user saw — so the bound applies to the tail below the page, which is where
/// an observation's size actually comes from.
fn candidate_cut(cfg: &Config, window: PageWindow, pooled: usize) -> usize {
    let page_end = if window.limit == 0 {
        pooled
    } else {
        window.offset.saturating_add(window.limit).min(pooled)
    };
    cfg.observations.max_candidates.max(page_end).min(pooled)
}

/// One contract candidate as a store row.
///
/// `unresolved` is `content_version().is_empty()`, which is exactly the state
/// `candidate_facts` leaves behind when a candidate's row vanished between
/// fusion and the text read: the code leg defaults `blob_oid` and the document
/// leg defaults `revision_hash` to the empty string, and a hit with no facts
/// entry at all gets a placeholder identity with an empty version. A memory
/// candidate always carries the digest of the body retrieval returned, and an
/// indexed code row always carries a blob OID — `index_code` skips a file it
/// cannot resolve one for — so neither produces a false positive. Deriving the
/// flag from a persisted column also lets a reader recompute it.
fn candidate_row<'a>(c: &'a CandidateObservation, locator_json: &'a str) -> NewCandidate<'a> {
    let content_version = c.identity.content_version();
    NewCandidate {
        pool_position: as_i64(c.pool_position),
        domain: c.identity.domain().as_str(),
        candidate_ref: &c.candidate_ref,
        content_version,
        unresolved: content_version.is_empty(),
        returned_position: c.returned_position.map(as_i64),
        retrieval_score: c.retrieval_score,
        rank_in_domain: as_i64(c.rank_in_domain),
        tier: c.tier.map(i64::from),
        text: &c.text.text,
        text_sha256: &c.text.sha256,
        text_full_bytes: as_i64(c.text.full_bytes),
        text_truncated: c.text.truncated,
        locator_json,
    }
}

/// A count as SQLite's signed integer, saturating rather than wrapping. Every
/// call site is a position or a length, so the ceiling is unreachable and the
/// saturating form exists to keep the conversion total.
fn as_i64(v: usize) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

#[cfg(test)]
#[path = "tests/observation_capture.rs"]
mod tests;
