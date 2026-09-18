//! Turn one fused candidate pool into the contract's own shapes: the
//! [`CandidateObservation`] list and the [`EffectiveFilters`] record.
//!
//! Split out of `benchmark_runner` so that file keeps the retrieval
//! orchestration and this one keeps the mapping onto the published contract.

use crate::domains::learning::evaluation::benchmark_set::{BenchmarkSet, BenchmarkTask};
use crate::domains::learning::evaluation::candidate_facts::FactsByHit;
use crate::domains::learning::evaluation::candidate_identity::{
    CandidateDomain, CandidateIdentity, CodeIdentity, DocumentIdentity, MemoryIdentity,
};
use crate::domains::learning::evaluation::candidate_observation::{
    BoundedText, CandidateLocator, CandidateObservation, EffectiveFilters, VectorScenario,
};
use crate::domains::retrieval::scope::{self, Domain, TimeScope};
use crate::domains::retrieval::unified::fuse_domains;

/// Pair each fused hit with its facts into a [`CandidateObservation`], and
/// count the candidates whose row vanished before their text could be read. A
/// hit with no facts entry keeps its pool position rather than being dropped.
pub fn observe(
    pool: &[fuse_domains::UnifiedHit],
    facts: &FactsByHit,
    k: usize,
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
                returned_position: (index < k).then_some(index + 1),
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

/// The complete filter record for one task, `null` where a dimension was not
/// narrowed. Time bounds are the normalized values the store compared against,
/// not the raw text the set carried.
pub fn effective_filters(
    task: &BenchmarkTask,
    set: &BenchmarkSet,
    time_scope: &TimeScope,
) -> EffectiveFilters {
    let mask = task.domain.mask();
    let domains = CandidateDomain::all()
        .into_iter()
        .filter(|d| {
            mask.contains(match d {
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
        repo: task.filters.repo.clone(),
        kind: task.filters.kind.clone(),
        lang: task.filters.lang.clone(),
        path_globs: task.filters.path.clone(),
        since: echo.since.map(str::to_string),
        until: echo.until.map(str::to_string),
        as_of: echo.as_of.map(str::to_string),
        vector: match (&set.vectors, &task.vector) {
            (Some(spec), Some(vector)) => VectorScenario::supplied(&spec.model, vector),
            _ => VectorScenario::Lexical,
        },
    }
}
