//! The document retrieval leg: BM25 over `document_fts`, chunk hits
//! coalesced to their parent document. Mirrors [`crate::domains::retrieval::router`]
//! and [`crate::domains::retrieval::code_route`] in spirit but stays thin — per the
//! design spec, documents carry no activation/feedback/quality/rank
//! inputs in v1, so this leg's order IS its lexical order.
//!
//! `--path` is a parameter here, not a [`Filters`] field: `Filters` is
//! `Copy` and a glob list is a `Vec`; this is the only v1 consumer.

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::domains::retrieval::scope::{Domain, Filters};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::document_fts::{self, DocumentFtsHit, HitSource};
use crate::store::documents;
use crate::store::remote_document_view;

/// Where a hit's text lives — the provenance a reader needs to know whether a
/// file backs the passage it is being shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocOrigin {
    /// A document indexed from a file on this machine.
    Local,
    /// A revision a peer shared, held in the pulled cache. There is no file
    /// behind it, which is exactly why the repository and the revision are
    /// carried: they are the only way to say which text this is.
    Shared {
        /// Canonical repository the revision belongs to.
        repo: String,
        /// The sender's `revision_hash` for the revision held here.
        revision_hash: String,
    },
}

/// One document-leg candidate: a document plus its single best-scoring
/// chunk, kept as the result's citation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocHit {
    /// The local `documents.id`, or the 32-hex `shared_id` when
    /// [`Self::origin`] is [`DocOrigin::Shared`] — a pulled revision has no
    /// local document row to name.
    pub document_id: String,
    /// Which side answered, and its provenance when a peer did.
    pub origin: DocOrigin,
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
    /// ordering (score desc, then id ascending — the local `documents.id` or
    /// the `shared_id`, whichever this hit carries, so the tie-break does not
    /// depend on which index answered).
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
            .then_with(|| source_id(&a.source).cmp(source_id(&b.source)))
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

/// Collapse `hits` to one [`DocumentFtsHit`] per document: the highest-scoring
/// chunk, ties broken toward the lower ordinal. Mirrors
/// [`crate::domains::retrieval::code_rerank`]'s parent-coalesce winner rule.
///
/// Keyed by the hit's SOURCE, so a local document and a pulled revision are
/// never coalesced into one another — the union already dropped the pulled
/// half of any document held on both sides, so two hits that reach here are
/// two different documents.
fn coalesce_to_documents(hits: Vec<DocumentFtsHit>) -> Vec<DocumentFtsHit> {
    let mut best: std::collections::BTreeMap<String, DocumentFtsHit> =
        std::collections::BTreeMap::new();
    for hit in hits {
        match best.entry(source_key(&hit.source)) {
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

/// One document's identity within its own index — what coalescing groups by.
///
/// The side is part of the GROUPING key because the two id spaces are
/// independent, but it is deliberately not part of the ORDERING key: prefixing
/// it would sort every local hit before every shared one at equal scores,
/// which is not the tie-break [`DocHit::bm25_rank`] documents.
fn source_key(source: &HitSource) -> String {
    match source {
        HitSource::Local(document_id) => format!("local:{document_id}"),
        HitSource::Shared { repo, shared_id } => format!("shared:{repo}:{shared_id}"),
    }
}

/// The bare id a hit carries, which is what an equal-score tie breaks on.
fn source_id(source: &HitSource) -> &str {
    match source {
        HitSource::Local(document_id) => document_id,
        HitSource::Shared { shared_id, .. } => shared_id,
    }
}

/// Resolve one coalesced FTS hit into a [`DocHit`], from whichever side
/// matched. `None` when the document or chunk vanished between the FTS match
/// and this read (a concurrent delete, re-index or tombstone), its repo does
/// not match `repo`, or its path does not match `matcher`.
fn build_hit(
    conn: &Connection,
    repo: Option<&str>,
    matcher: Option<&GlobSet>,
    hit: DocumentFtsHit,
    bm25_rank: usize,
) -> Result<Option<DocHit>> {
    let found = match &hit.source {
        HitSource::Local(document_id) => local_hit(conn, document_id, hit.ordinal)?,
        HitSource::Shared { repo, shared_id } => shared_hit(conn, repo, shared_id, hit.ordinal)?,
    };
    let Some(found) = found else {
        return Ok(None);
    };
    if let Some(want) = repo
        && found.repo.as_deref() != Some(want)
    {
        return Ok(None);
    }
    if matcher.is_some_and(|m| !m.is_match(&found.path)) {
        return Ok(None);
    }
    Ok(Some(DocHit {
        document_id: found.id,
        origin: found.origin,
        title: found.title,
        path: found.path,
        chunk_ordinal: hit.ordinal,
        heading_path: found.citation.0,
        line_range: found.citation.1,
        snippet: found.citation.2,
        bm25_rank,
    }))
}

/// What a resolved hit contributes, whichever side it came from.
struct Resolved {
    /// The local `documents.id`, or the `shared_id` of a pulled revision.
    id: String,
    /// Which side answered.
    origin: DocOrigin,
    /// The repository the document belongs to, when it has one.
    repo: Option<String>,
    /// Document title.
    title: String,
    /// The path a citation points at.
    path: String,
    /// The winning chunk's breadcrumb, line range and text.
    citation: (String, (i64, i64), String),
}

/// Resolve a locally indexed document's hit. `None` when the document, its
/// path or the chunk vanished between the match and this read.
fn local_hit(conn: &Connection, document_id: &str, ordinal: i64) -> Result<Option<Resolved>> {
    let (Some(doc), Some(path), Some(chunk)) = (
        documents::get_document(conn, document_id)?,
        documents::get_document_path(conn, document_id)?,
        documents::get_chunk(conn, document_id, ordinal)?,
    ) else {
        return Ok(None);
    };
    Ok(Some(Resolved {
        id: document_id.to_string(),
        origin: DocOrigin::Local,
        repo: doc.repo,
        title: doc.title,
        path,
        citation: (chunk.heading_path, chunk.line_range, chunk.text),
    }))
}

/// Resolve a pulled revision's hit.
///
/// Its path is relative to the REPOSITORY, where a local hit's is relative to
/// its registered source root — `docs/guides/x.md` against `x.md` for a source
/// rooted at `docs/guides`. A `--path` glob therefore sees two shapes, and one
/// written for local hits may not match a pulled one. Normalizing them would
/// mean inventing a source root for a document that has no file here.
fn shared_hit(
    conn: &Connection,
    repo: &str,
    shared_id: &str,
    ordinal: i64,
) -> Result<Option<Resolved>> {
    let (Some(doc), Some(chunk)) = (
        remote_document_view::shared_document(conn, repo, shared_id)?,
        remote_document_view::shared_passage(conn, repo, shared_id, ordinal)?,
    ) else {
        return Ok(None);
    };
    Ok(Some(Resolved {
        id: doc.shared_id,
        origin: DocOrigin::Shared {
            repo: doc.repo.clone(),
            revision_hash: doc.revision_hash,
        },
        repo: Some(doc.repo),
        title: doc.title,
        path: doc.path,
        citation: (chunk.heading_path, chunk.line_range, chunk.text),
    }))
}

#[cfg(test)]
#[path = "tests/doc_route.rs"]
mod tests;
