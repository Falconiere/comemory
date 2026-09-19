//! Turn one fused candidate pool into the contract's own shapes: the
//! [`CandidateObservation`] list and the [`EffectiveFilters`] record.
//!
//! Split out of `benchmark_runner` so that file keeps the retrieval
//! orchestration and this one keeps the mapping onto the published contract.

use crate::domains::learning::evaluation::candidate_facts::FactsByHit;
use crate::domains::learning::evaluation::candidate_identity::{
    CandidateDomain, CandidateIdentity, CodeIdentity, DocumentIdentity, MemoryIdentity,
};
use crate::domains::learning::evaluation::candidate_observation::{
    BoundedText, CandidateLocator, CandidateObservation, EffectiveFilters, VectorScenario,
};
use crate::domains::retrieval::scope::{self, Domain, Domains, TimeScope};
use crate::domains::retrieval::unified::fuse_domains;
use crate::utilities::pagination::PageWindow;

/// Pair each fused hit with its facts into a [`CandidateObservation`], and
/// count the candidates that reached the pool with no text.
///
/// That covers both ways a row can go missing between fusion and the facts
/// read: no facts entry at all, and an entry whose own row vanished
/// (`text_available == false`, e.g. a document whose parent was deleted). Both
/// mean "observed, but nothing to score", and a hit in either state keeps its
/// pool position rather than being dropped — dropping it would silently shrink
/// the pool, which is what pool recall exists to measure.
pub fn observe(
    pool: &[fuse_domains::UnifiedHit],
    facts: &FactsByHit,
    window: PageWindow,
) -> (Vec<CandidateObservation>, usize) {
    let mut unavailable = 0usize;
    let candidates = pool
        .iter()
        .enumerate()
        .map(|(index, hit)| {
            let domain = domain_of(&hit.domain);
            let entry = facts.get(&(domain, hit.id.clone()));
            if !entry.is_some_and(|f| f.text_available) {
                unavailable += 1;
            }
            let identity = entry.map_or_else(
                || placeholder_identity(domain, &hit.id),
                |f| f.identity.clone(),
            );
            CandidateObservation {
                candidate_ref: identity.candidate_ref(),
                identity,
                pool_position: index + 1,
                returned_position: returned_position(window, index),
                retrieval_score: hit.score,
                rank_in_domain: hit.rank_in_domain,
                tier: hit.tier,
                text: entry.map_or_else(BoundedText::unavailable, |f| f.text.clone()),
                locator: CandidateLocator {
                    title: hit.title.clone(),
                    repo: hit.repo.clone(),
                    path: hit.path.clone(),
                    line_range: entry.and_then(|f| f.line_range),
                    heading_path: entry.and_then(|f| f.heading_path.clone()),
                    symbol_id: entry.and_then(|f| f.symbol_id),
                },
            }
        })
        .collect();
    (candidates, unavailable)
}

/// Where a pooled candidate at `index` landed on the displayed page, or
/// `None` when it fell outside it.
///
/// Mirrors `pipeline::paginate` exactly, including its `limit == 0` case
/// ("everything from the offset onward"), so a candidate is reported as
/// returned precisely when the caller was handed it. A benchmark run passes
/// `offset: 0`; a real `find` may not, which is why the contract carries
/// `page_offset` at all.
fn returned_position(window: PageWindow, index: usize) -> Option<usize> {
    if index < window.offset {
        return None;
    }
    let within = window.limit == 0 || index < window.offset.saturating_add(window.limit);
    within.then(|| index - window.offset + 1)
}

/// The candidate domain a fused hit's label names. `fuse_domains` emits only
/// the three labels, so an unknown one falls back to memory rather than
/// silently dropping the candidate out of the pool.
fn domain_of(label: &str) -> CandidateDomain {
    match label {
        fuse_domains::DOMAIN_CODE => CandidateDomain::Code,
        fuse_domains::DOMAIN_DOCUMENT => CandidateDomain::Document,
        _ => CandidateDomain::Memory,
    }
}

/// The identity a candidate gets when its row vanished before its facts could
/// be read: the id retrieval reported, with an empty content version.
fn placeholder_identity(domain: CandidateDomain, id: &str) -> CandidateIdentity {
    match domain {
        CandidateDomain::Memory => CandidateIdentity::Memory(MemoryIdentity {
            memory_id: id.to_string(),
            content_hash: String::new(),
        }),
        CandidateDomain::Code => CandidateIdentity::Code(CodeIdentity {
            repo: String::new(),
            path: String::new(),
            symbol: id.to_string(),
            blob_oid: String::new(),
        }),
        CandidateDomain::Document => CandidateIdentity::Document(DocumentIdentity {
            document_id: id.to_string(),
            path: String::new(),
            revision_hash: String::new(),
            chunk_ordinal: -1,
        }),
    }
}

/// The narrowing one run applied, as the caller already holds it — a
/// benchmark task and a `find` request supply the same five values from
/// different shapes, and this is where they meet so the mapping onto
/// [`EffectiveFilters`] has exactly one implementation.
#[derive(Debug, Clone, Copy)]
pub struct FilterInputs<'a> {
    /// The legs that were in scope.
    pub domains: Domains,
    /// Repo label; narrows the memory and code legs.
    pub repo: Option<&'a str>,
    /// Canonical lowercase memory kind; narrows the memory leg only.
    pub kind: Option<&'a str>,
    /// Source language; narrows the code leg only.
    pub lang: Option<&'a str>,
    /// Git-style path globs; narrow the document leg only.
    pub path_globs: &'a [String],
}

/// The complete filter record for one run, `null` where a dimension was not
/// narrowed. Time bounds are the normalized values the store compared
/// against, not the raw text the caller supplied.
pub fn effective_filters(
    inputs: FilterInputs<'_>,
    time_scope: &TimeScope,
    vector: VectorScenario,
) -> EffectiveFilters {
    let domains = CandidateDomain::all()
        .into_iter()
        .filter(|d| {
            inputs.domains.contains(match d {
                CandidateDomain::Memory => Domain::Memory,
                CandidateDomain::Code => Domain::Code,
                CandidateDomain::Document => Domain::Document,
            })
        })
        .map(|d| d.as_str().to_string())
        .collect();
    let echo = scope::ScopeEcho::of(time_scope);
    EffectiveFilters {
        domains,
        repo: inputs.repo.map(str::to_string),
        kind: inputs.kind.map(str::to_string),
        lang: inputs.lang.map(str::to_string),
        path_globs: inputs.path_globs.to_vec(),
        since: echo.since.map(str::to_string),
        until: echo.until.map(str::to_string),
        as_of: echo.as_of.map(str::to_string),
        vector,
    }
}
