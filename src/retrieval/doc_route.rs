//! The document retrieval leg: BM25 over `document_fts`, chunk hits
//! coalesced to their parent document. Mirrors [`crate::retrieval::router`]
//! and [`crate::retrieval::code_route`] in spirit but stays thin — per the
//! design spec, documents carry no activation/feedback/quality/rank
//! inputs in v1, so this leg's order IS its lexical order.
//!
//! `--path` is a parameter here, not a [`Filters`] field: `Filters` is
//! `Copy` and a glob list is a `Vec`; this is the only v1 consumer.

use globset::{Glob, GlobSet, GlobSetBuilder};
use rusqlite::Connection;

use crate::prelude::*;
use crate::retrieval::scope::{Domain, Filters};
use crate::store::document_fts::{self, DocumentFtsHit};
use crate::store::documents;

/// One document-leg candidate: a document plus its single best-scoring
/// chunk, kept as the result's citation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocHit {
    /// 32-hex-char `documents.id`.
    pub document_id: String,
    /// Document title (first heading, else file stem).
    pub title: String,
    /// Path relative to the owning source root (`source_files.relative_path`).
    pub path: String,
    /// 0-based ordinal of the winning chunk within the document.
    pub chunk_ordinal: i64,
    /// `" > "`-joined heading breadcrumb of the winning chunk.
    pub heading_path: String,
    /// `(start, end)` inclusive 1-based line range of the winning chunk.
    pub line_range: (i64, i64),
    /// The winning chunk's raw passage text — the result's snippet.
    pub snippet: String,
    /// 1-based position of this document within the leg's own BM25
    /// ordering (score desc, document id tie-break).
    pub bm25_rank: usize,
}

/// Route a document query: BM25 over `document_fts`, coalesced to one hit
/// per parent document (highest-scoring chunk wins, ties toward the lower
/// ordinal), ordered by that score with document id as tie-break.
///
/// Empty without touching the database when the domain scope excludes
/// [`Domain::Document`] or the query is blank. `k` is the FTS candidate
/// pool (chunk granularity); coalescing can shrink the returned count
/// below it. `path_globs` narrows to documents whose relative path
/// matches at least one entry (OR semantics; empty = no filter); an
/// unparsable glob is a usage error naming the pattern.
pub fn route_documents(
    conn: &Connection,
    query: &str,
    filters: Filters<'_>,
    path_globs: &[String],
    k: usize,
) -> Result<Vec<DocHit>> {
    if !filters.domains.contains(Domain::Document) || query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let matcher = build_path_matcher(path_globs)?;
    let mut winners = coalesce_to_documents(document_fts::search(conn, query, k)?);
    winners.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.document_id.cmp(&b.document_id))
    });
    let mut out = Vec::with_capacity(winners.len());
    for (rank, hit) in winners.into_iter().enumerate() {
        if let Some(doc_hit) = build_hit(conn, filters.repo, matcher.as_ref(), hit, rank + 1)? {
            out.push(doc_hit);
        }
    }
    Ok(out)
}

/// Compile `globs` (Git-style patterns, OR'd together) into one matcher,
/// or `None` when `globs` is empty (no `--path` filtering requested). An
/// unparsable pattern is rejected as [`Error::Usage`] naming the pattern,
/// not silently dropped.
fn build_path_matcher(globs: &[String]) -> Result<Option<GlobSet>> {
    if globs.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in globs {
        let glob = Glob::new(pattern)
            .map_err(|e| Error::Usage(format!("--path: invalid glob `{pattern}`: {e}")))?;
        builder.add(glob);
    }
    let set = builder
        .build()
        .map_err(|e| Error::Usage(format!("--path: invalid glob pattern set: {e}")))?;
    Ok(Some(set))
}

/// Collapse `hits` to one [`DocumentFtsHit`] per `document_id`: the
/// highest-scoring chunk, ties broken toward the lower ordinal. Mirrors
/// [`crate::retrieval::code_rerank`]'s parent-coalesce winner rule.
fn coalesce_to_documents(hits: Vec<DocumentFtsHit>) -> Vec<DocumentFtsHit> {
    let mut best: std::collections::BTreeMap<String, DocumentFtsHit> =
        std::collections::BTreeMap::new();
    for hit in hits {
        match best.entry(hit.document_id.clone()) {
            std::collections::btree_map::Entry::Vacant(v) => {
                v.insert(hit);
            }
            std::collections::btree_map::Entry::Occupied(mut o) => {
                if wins(&hit, o.get()) {
                    o.insert(hit);
                }
            }
        }
    }
    best.into_values().collect()
}

/// Whether `candidate` should replace `incumbent` as its document's
/// winning chunk: a strictly higher score always wins; an exact tie goes
/// to the lower ordinal so coalescing is deterministic.
fn wins(candidate: &DocumentFtsHit, incumbent: &DocumentFtsHit) -> bool {
    match candidate.score.total_cmp(&incumbent.score) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Equal => candidate.ordinal < incumbent.ordinal,
        std::cmp::Ordering::Less => false,
    }
}

/// Resolve one coalesced FTS hit into a [`DocHit`]: fetch the owning
/// `documents` row (applying the `repo` filter), its relative path
/// (applying `matcher`, if any), then the winning chunk's citation
/// fields. `None` when the document or chunk vanished between the FTS
/// match and this read (a concurrent delete/re-index), the document's
/// repo does not match `repo`, or its path does not match `matcher`.
fn build_hit(
    conn: &Connection,
    repo: Option<&str>,
    matcher: Option<&GlobSet>,
    hit: DocumentFtsHit,
    bm25_rank: usize,
) -> Result<Option<DocHit>> {
    let Some(doc) = documents::get_document(conn, &hit.document_id)? else {
        return Ok(None);
    };
    if let Some(want) = repo
        && doc.repo.as_deref() != Some(want)
    {
        return Ok(None);
    }
    let Some(path) = documents::get_document_path(conn, &hit.document_id)? else {
        return Ok(None);
    };
    if matcher.is_some_and(|m| !m.is_match(&path)) {
        return Ok(None);
    }
    let Some(chunk) = documents::get_chunk(conn, &hit.document_id, hit.ordinal)? else {
        return Ok(None);
    };
    Ok(Some(DocHit {
        document_id: hit.document_id,
        title: doc.title,
        path,
        chunk_ordinal: hit.ordinal,
        heading_path: chunk.heading_path,
        line_range: chunk.line_range,
        snippet: chunk.text,
        bm25_rank,
    }))
}

#[cfg(test)]
#[path = "tests/doc_route.rs"]
mod tests;
