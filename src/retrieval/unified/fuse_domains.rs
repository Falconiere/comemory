//! Weighted N-ary fusion across the three retrieval domains, and the
//! domain-tagged hit assembly that turns each leg's own row type into one
//! comparable [`UnifiedHit`].
//!
//! Split out of `retrieval::unified` so that module keeps only the entry
//! point and the pagination rule; everything about *combining* the legs
//! lives here.
//!
//! The fusion is [`fuse::rrf_multi_weighted`] — already in the crate, and
//! already bit-identical to the unweighted `rrf_multi` when every weight is
//! `1.0`. Memory and code fuse at weight `1.0`; the document leg fuses at
//! `cfg.retrieval.document_leg_weight`. That config key has existed and been
//! validated since the document domain landed, and until now was read by
//! nothing — this module is what finally consumes it.

use std::collections::HashMap;

use serde::Serialize;

use crate::retrieval::code_rerank::{CodeReranked, CodeScoreParts};
use crate::retrieval::doc_route::DocHit;
use crate::retrieval::fuse::{self, RankedHit};
use crate::retrieval::rerank::{Reranked, ScoreParts};
use crate::store::memory_meta::MemoryMeta;

/// Domain label carried by a memory hit.
pub const DOMAIN_MEMORY: &str = "memory";
/// Domain label carried by a code hit.
pub const DOMAIN_CODE: &str = "code";
/// Domain label carried by a document hit.
pub const DOMAIN_DOCUMENT: &str = "document";

/// The domain's own explainability object, typed rather than an untyped
/// `serde_json::Value` so the parity walk and `deny_unknown_fields` can
/// still see through it. `untagged` keeps the serialized shape identical to
/// what each domain's dedicated command already emits — a consumer that
/// knows `domain` knows which variant it is holding.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum HitParts {
    /// A memory hit's five multiplicative priors plus its leg scores.
    Memory(Box<ScoreParts>),
    /// A code hit's four priors plus its leg scores.
    Code(Box<CodeScoreParts>),
    /// A document hit's BM25 position. The document leg has no rerank
    /// stage of its own, so there are no priors to report.
    Document(DocParts),
}

/// The document leg's explainability object. Deliberately thin: the leg is
/// BM25-only today, and inventing neutral `1.0` priors it never applies
/// would misrepresent it as having a rerank stage.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DocParts {
    /// 1-based position within the document leg's own BM25 ordering.
    pub bm25_rank: usize,
}

/// One hit in the unified ranking, tagged with the domain it came from.
#[derive(Debug, Clone, Serialize)]
pub struct UnifiedHit {
    /// `memory` | `code` | `document`.
    pub domain: String,
    /// Memory id, `code_symbols` id, or document id — the identifier that
    /// domain's own commands use.
    pub id: String,
    /// Human-readable headline.
    pub title: String,
    /// The dim second line: kind/repo for a memory, symbol/lang for code,
    /// heading path for a document.
    pub subtitle: String,
    /// Owning repo, where the domain has one.
    pub repo: Option<String>,
    /// File path, where the domain has one.
    pub path: Option<String>,
    /// Fused score across every leg.
    pub score: f64,
    /// 1-based position within this hit's OWN domain, before fusion. Lets a
    /// caller filter to one domain and still see that domain's true order.
    pub rank_in_domain: usize,
    /// The domain's own score breakdown, verbatim.
    pub score_parts: HitParts,
}

/// The three legs' reranked output, each still in its own domain's order.
pub struct Legs<'a> {
    /// Memory hits, already reranked and diversified.
    pub memory: Vec<Reranked>,
    /// Batched navigation metadata for `memory`, keyed by memory id.
    pub memory_meta: &'a HashMap<String, MemoryMeta>,
    /// Code hits, already reranked.
    pub code: Vec<CodeReranked>,
    /// Document hits, in BM25 order.
    pub documents: Vec<DocHit>,
}

/// Fuse the three legs into one ranking.
///
/// Each leg is fused **in its own already-reranked order**, which is what
/// makes a `--domain memory` run order-identical to `comemory search` and a
/// `--domain code` run order-identical to `comemory search-code`: RRF reads
/// ranks, so preserving a leg's internal order preserves its output when it
/// is the only leg.
///
/// Ids are namespaced (`memory:…`, `code:…`, `document:…`) before fusion
/// because RRF accumulates into one id-keyed map, and a `code_symbols`
/// rowid could otherwise collide with a document ordinal.
pub fn fuse(legs: Legs<'_>, doc_weight: f32, pool: usize, rrf_k: f32) -> Vec<UnifiedHit> {
    let memory_ranked = ranked_list(DOMAIN_MEMORY, legs.memory.iter().map(|h| &h.memory_id));
    let code_ranked = ranked_list(
        DOMAIN_CODE,
        legs.code
            .iter()
            .map(|h| h.symbol_id.to_string())
            .collect::<Vec<_>>()
            .iter(),
    );
    let doc_ranked = ranked_list(
        DOMAIN_DOCUMENT,
        legs.documents.iter().map(|h| &h.document_id),
    );

    let weighted: Vec<(&[RankedHit], f32)> = vec![
        (memory_ranked.as_slice(), 1.0),
        (code_ranked.as_slice(), 1.0),
        (doc_ranked.as_slice(), doc_weight),
    ];
    let fused = fuse::rrf_multi_weighted(&weighted, pool, rrf_k);

    let mut by_id: HashMap<String, UnifiedHit> = HashMap::new();
    for (i, h) in legs.memory.into_iter().enumerate() {
        by_id.insert(
            namespaced(DOMAIN_MEMORY, &h.memory_id),
            memory_hit(h, i + 1, legs.memory_meta),
        );
    }
    for (i, h) in legs.code.into_iter().enumerate() {
        by_id.insert(
            namespaced(DOMAIN_CODE, &h.symbol_id.to_string()),
            code_hit(h, i + 1),
        );
    }
    for (i, h) in legs.documents.into_iter().enumerate() {
        by_id.insert(
            namespaced(DOMAIN_DOCUMENT, &h.document_id),
            doc_hit(h, i + 1),
        );
    }

    fused
        .into_iter()
        .filter_map(|r| {
            by_id.remove(&r.memory_id).map(|mut hit| {
                hit.score = f64::from(r.score);
                hit
            })
        })
        .collect()
}

/// `<domain>:<id>` — the fusion key. See [`fuse`] for why ids are
/// namespaced before they enter the accumulator.
fn namespaced(domain: &str, id: &str) -> String {
    format!("{domain}:{id}")
}

/// Turn one leg's id sequence into the [`RankedHit`] list RRF consumes.
/// The score is not read by RRF (it ranks by position), so the leg's own
/// order is the entire input.
fn ranked_list<'a>(domain: &str, ids: impl Iterator<Item = &'a String>) -> Vec<RankedHit> {
    ids.enumerate()
        .map(|(i, id)| RankedHit {
            memory_id: namespaced(domain, id),
            score: 1.0 / (i as f32 + 1.0),
        })
        .collect()
}

/// Shape a reranked memory into a [`UnifiedHit`], reading its navigation
/// metadata for the title and repo the console shows.
fn memory_hit(h: Reranked, rank: usize, meta: &HashMap<String, MemoryMeta>) -> UnifiedHit {
    let entry = meta.get(&h.memory_id);
    let kind = entry.map(|m| m.kind.as_str()).unwrap_or_default();
    let repo = entry.and_then(|m| m.repo.clone());
    let refs = entry.map_or(0, |m| m.references.files.len() + m.references.symbols.len());
    let title = crate::output::search::title_of(&h.body);
    let subtitle = match &repo {
        Some(r) => format!("{} · {kind} · {r} · {refs} refs", h.memory_id),
        None => format!("{} · {kind} · {refs} refs", h.memory_id),
    };
    UnifiedHit {
        domain: DOMAIN_MEMORY.to_string(),
        id: h.memory_id,
        title,
        subtitle,
        repo,
        path: entry.map(|m| m.md_path.clone()),
        score: 0.0,
        rank_in_domain: rank,
        score_parts: HitParts::Memory(Box::new(h.parts)),
    }
}

/// Shape a reranked code symbol into a [`UnifiedHit`].
fn code_hit(h: CodeReranked, rank: usize) -> UnifiedHit {
    UnifiedHit {
        domain: DOMAIN_CODE.to_string(),
        id: h.symbol_id.to_string(),
        title: h.path.clone(),
        subtitle: format!("{} · {} · {}", h.symbol, h.kind, h.lang),
        repo: Some(h.repo),
        path: Some(h.path),
        score: 0.0,
        rank_in_domain: rank,
        score_parts: HitParts::Code(Box::new(h.parts)),
    }
}

/// Shape a document hit into a [`UnifiedHit`].
fn doc_hit(h: DocHit, rank: usize) -> UnifiedHit {
    UnifiedHit {
        domain: DOMAIN_DOCUMENT.to_string(),
        id: h.document_id,
        title: h.title,
        subtitle: if h.heading_path.is_empty() {
            h.path.clone()
        } else {
            format!("{} · {}", h.path, h.heading_path)
        },
        repo: None,
        path: Some(h.path),
        score: 0.0,
        rank_in_domain: rank,
        score_parts: HitParts::Document(DocParts {
            bm25_rank: h.bm25_rank,
        }),
    }
}
