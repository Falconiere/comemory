//! Turn one run's leg rows into the identity, content version and bounded text
//! a candidate observation needs — the fields `fuse_domains::UnifiedHit` drops.
//!
//! Runs before fusion, borrowing the rows `unified::run_legs` returned, so the
//! benchmark reads the same candidates `find` fuses rather than re-querying.
//! A candidate whose row vanished between fusion and this read keeps its place
//! with empty text: dropping it would silently shrink the pool, which is what
//! pool recall exists to measure.

use std::collections::HashMap;

use crate::domains::learning::evaluation::candidate_identity::{
    CandidateDomain, CandidateIdentity, CodeIdentity, DocumentIdentity, MemoryIdentity,
};
use crate::domains::learning::evaluation::candidate_observation::BoundedText;
use crate::domains::retrieval::code_rerank::CodeReranked;
use crate::domains::retrieval::doc_route::DocHit;
use crate::domains::retrieval::rerank::Reranked;
use crate::domains::retrieval::unified::LegRows;
use crate::prelude::*;
use crate::store::{Connection, code_text, documents};
use crate::utilities::digest::sha256_hex;

/// Everything about one candidate that its domain's hit shape does not carry.
#[derive(Debug, Clone)]
pub struct CandidateFacts {
    /// Domain-qualified stable identity plus content version.
    pub identity: CandidateIdentity,
    /// The candidate text, bounded.
    pub text: BoundedText,
    /// 1-based inclusive line range, for code and document candidates.
    pub line_range: Option<(i64, i64)>,
    /// Heading breadcrumb, document candidates only.
    pub heading_path: Option<String>,
    /// `code_symbols.id`, a session-scoped locator and never an identity.
    pub symbol_id: Option<i64>,
    /// Whether the candidate's row was still there to read text from.
    pub text_available: bool,
}

/// Facts keyed exactly as `UnifiedHit` keys a candidate: its domain plus the id
/// that domain's own commands print.
pub type FactsByHit = HashMap<(CandidateDomain, String), CandidateFacts>;

/// One leg's hit paired with whatever extra row the store had to supply for
/// it. Exists so every domain's [`CandidateFacts`] is built at ONE site: three
/// near-identical per-domain constructors is the duplication this collapses.
enum LegHit<'a> {
    /// A memory hit; its body is both the text and the content version.
    Memory(&'a Reranked),
    /// A code hit plus its `code_symbols` row, absent when the row vanished.
    Code(&'a CodeReranked, Option<&'a code_text::CodeText>),
    /// A document hit plus its parent's revision, absent when the row vanished.
    Document(&'a DocHit, Option<&'a str>),
}

/// Collect facts for every candidate in `legs`, bounding each text at
/// `max_text_bytes`.
///
/// One batched read per leg that needs one: `code_symbols` rows for the code
/// leg, `documents.revision_hash` for the document leg. The search path pays
/// neither, which is why fusion drops these fields in the first place.
pub fn collect(conn: &Connection, legs: &LegRows, max_text_bytes: usize) -> Result<FactsByHit> {
    let code_rows = code_text::fetch(
        conn,
        &legs.code.iter().map(|h| h.symbol_id).collect::<Vec<_>>(),
    )?;
    let document_ids: Vec<&str> = legs
        .documents
        .iter()
        .map(|h| h.document_id.as_str())
        .collect();
    let revisions = documents::fetch_revisions(conn, &document_ids)?;
    let memory = legs.memory.iter().map(LegHit::Memory);
    let code = legs
        .code
        .iter()
        .map(|hit| LegHit::Code(hit, code_rows.get(&hit.symbol_id)));
    let documents = legs
        .documents
        .iter()
        .map(|hit| LegHit::Document(hit, revisions.get(&hit.document_id).map(String::as_str)));
    Ok(memory
        .chain(code)
        .chain(documents)
        .map(|hit| facts_of(hit, max_text_bytes))
        .collect())
}

/// The keyed facts for one hit, whatever leg it came from.
///
/// Per-domain rules, all decided here: a memory's content version is
/// `sha256(body.trim_end())` over the body retrieval returned, which is by
/// construction what `memories.content_hash` and the frontmatter carry. A code
/// candidate's identity, content version and text all come from the row at
/// `symbol_id` — the parent, for a coalesced cAST chunk — while `line_range`
/// stays the winning chunk's, exactly as `comemory search-code` reports it; the
/// two are allowed to disagree. A document's identity is its parent plus the
/// winning chunk's ordinal, and its text is that chunk's passage.
fn facts_of(hit: LegHit<'_>, max_text_bytes: usize) -> ((CandidateDomain, String), CandidateFacts) {
    match hit {
        LegHit::Memory(m) => (
            (CandidateDomain::Memory, m.memory_id.clone()),
            CandidateFacts {
                identity: CandidateIdentity::Memory(MemoryIdentity {
                    memory_id: m.memory_id.clone(),
                    content_hash: sha256_hex(m.body.trim_end().as_bytes()),
                }),
                text: BoundedText::bound(&m.body, max_text_bytes),
                line_range: None,
                heading_path: None,
                symbol_id: None,
                text_available: true,
            },
        ),
        LegHit::Code(c, row) => (
            (CandidateDomain::Code, c.symbol_id.to_string()),
            CandidateFacts {
                identity: CandidateIdentity::Code(CodeIdentity {
                    repo: c.repo.clone(),
                    path: c.path.clone(),
                    symbol: c.symbol.clone(),
                    blob_oid: row.map(|r| r.blob_oid.clone()).unwrap_or_default(),
                }),
                text: row.map_or_else(BoundedText::unavailable, |r| {
                    BoundedText::bound(&r.snippet, max_text_bytes)
                }),
                line_range: Some((c.line_start, c.line_end)),
                heading_path: None,
                symbol_id: Some(c.symbol_id),
                text_available: row.is_some(),
            },
        ),
        LegHit::Document(d, revision) => (
            (CandidateDomain::Document, d.document_id.clone()),
            CandidateFacts {
                identity: CandidateIdentity::Document(DocumentIdentity {
                    document_id: d.document_id.clone(),
                    path: d.path.clone(),
                    revision_hash: revision.unwrap_or_default().to_string(),
                    chunk_ordinal: d.chunk_ordinal,
                }),
                text: BoundedText::bound(&d.snippet, max_text_bytes),
                line_range: Some(d.line_range),
                heading_path: Some(d.heading_path.clone()),
                symbol_id: None,
                text_available: revision.is_some(),
            },
        ),
    }
}
