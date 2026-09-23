//! The `--only` clap surface, plus the interim `--only document` search
//! path for `comemory search`. The resolution policy itself is
//! transport-neutral and lives in
//! [`crate::domains::retrieval::scope::resolve_domains`]; this module only
//! maps clap's own enum onto it.
//!
//! s9 fuses the document leg into `retrieval::pipeline` with a pinned
//! `domain` + `fused_score` row shape (`output::document`); until then a
//! scope excluding memory can't go through `pipeline::search` (it always
//! runs the memory leg regardless of `Filters.domains`). This module runs
//! `doc_route::route_documents` directly and renders a small interim
//! envelope (`domain`, `document_id`, `title`, `snippet`, `citation`; no
//! `fused_score`/`score_parts` yet). Documents stay outside the learning
//! loop (spec Non-Goal 9): no access tracking, no `retrieval_log` row.

use clap::ValueEnum;
use serde::Serialize;

use crate::cli::output::{json, tty};
use crate::config::Config;
use crate::domains::retrieval::doc_route::{self, DocHit, DocOrigin};
use crate::domains::retrieval::pipeline;
use crate::domains::retrieval::scope::{self, Domain, Domains, Filters};
use crate::prelude::*;
use crate::store::Connection;
use crate::utilities::pagination::Page;
use crate::utilities::pagination::PageWindow;

/// One CLI-selectable retrieval domain for `--only`. Mirrors [`Domain`]'s
/// three variants; kept as its own `clap::ValueEnum` so `--only`'s
/// comma-separated/repeatable parsing and its invalid-value error
/// (naming the valid choices) stay on clap's native path — the same
/// precedent as `--kind` (`memory::Kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum OnlyDomain {
    /// Hand-authored markdown memories.
    Memory,
    /// Indexed external documents.
    Document,
    /// Extracted code symbols.
    Code,
}

impl From<OnlyDomain> for Domain {
    fn from(d: OnlyDomain) -> Self {
        match d {
            OnlyDomain::Memory => Domain::Memory,
            OnlyDomain::Document => Domain::Document,
            OnlyDomain::Code => Domain::Code,
        }
    }
}

/// Map the raw clap `--only` values onto [`Domain`] and hand them to
/// [`scope::resolve_domains`], which owns the policy and its usage errors.
pub fn resolve_domains(only: &[OnlyDomain], kind: Option<&str>) -> Result<Domains> {
    let domains: Vec<Domain> = only.iter().map(|d| Domain::from(*d)).collect();
    scope::resolve_domains(&domains, kind)
}

/// Run the interim document-only search path: `doc_route` directly, no
/// shared pipeline. `filters.domains` must already exclude
/// [`Domain::Memory`] — the caller decides that branch (see
/// `cli::search::run`).
pub fn run_document_only(
    conn: &Connection,
    cfg: &Config,
    query: &str,
    filters: Filters<'_>,
    path_globs: &[String],
    window: PageWindow,
    json_flag: bool,
) -> Result<()> {
    let pool = pipeline::pool_size(window.offset, window.limit, cfg.retrieval.max_page_window);
    let hits = doc_route::route_documents(conn, query, filters, path_globs, pool)?;
    let page = Page::from_slice(hits, window.limit, window.offset);
    if json_flag {
        let rows: Vec<DocRow<'_>> = page.items.iter().map(doc_row).collect();
        return json::write(&Page::new(
            rows,
            page.limit,
            page.offset,
            page.total,
            page.has_more,
        ));
    }
    write_tty(&mut std::io::stdout().lock(), &page)
}

/// One document hit as emitted under this interim path. See the module
/// docs for why this is smaller than the pinned `output::document` shape.
#[derive(Serialize)]
struct DocRow<'a> {
    /// Always `"document"` — the field s9 generalizes across every domain.
    domain: &'static str,
    /// 32-hex-char `documents.id`.
    document_id: &'a str,
    /// Document title.
    title: &'a str,
    /// The winning chunk's passage text.
    snippet: &'a str,
    /// `"local"` for a document indexed from a file here, else the canonical
    /// repository a peer shared it from.
    shared_from: Option<&'a str>,
    /// The sender's revision of a shared document; absent for a local one.
    revision: Option<&'a str>,
    /// Where the snippet came from.
    citation: Citation<'a>,
}

/// A document hit's provenance: enough to open the exact passage.
#[derive(Serialize)]
struct Citation<'a> {
    /// Path relative to the owning source root.
    path: &'a str,
    /// `" > "`-joined heading breadcrumb.
    heading_path: &'a str,
    /// Inclusive 1-based first line of the passage.
    line_start: i64,
    /// Inclusive 1-based last line of the passage.
    line_end: i64,
}

fn doc_row(h: &DocHit) -> DocRow<'_> {
    let (shared_from, revision) = match &h.origin {
        DocOrigin::Local => (None, None),
        DocOrigin::Shared {
            repo,
            revision_hash,
        } => (Some(repo.as_str()), Some(revision_hash.as_str())),
    };
    DocRow {
        domain: "document",
        document_id: &h.document_id,
        title: &h.title,
        snippet: &h.snippet,
        shared_from,
        revision,
        citation: Citation {
            path: &h.path,
            heading_path: &h.heading_path,
            line_start: h.line_range.0,
            line_end: h.line_range.1,
        },
    }
}

/// Render the TTY view of a document-only page: one title/citation line
/// per hit, dim snippet beneath, then the shared page footer.
fn write_tty(out: &mut impl std::io::Write, page: &Page<DocHit>) -> Result<()> {
    for hit in &page.items {
        // A shared passage has no file behind it on this machine, so saying
        // where it came from is the difference between a citation an operator
        // can open and one they cannot.
        let from = match &hit.origin {
            DocOrigin::Local => String::new(),
            DocOrigin::Shared { repo, .. } => format!("  shared from {repo}"),
        };
        writeln!(
            out,
            "{}  {} ({}:{}-{}){}",
            hit.document_id, hit.title, hit.path, hit.line_range.0, hit.line_range.1, from
        )?;
        writeln!(out, "{}", tty::dim(&hit.snippet))?;
    }
    tty::write_page_footer(out, page.items.len(), page.offset, page.total)
}

#[cfg(test)]
#[path = "tests/search_only.rs"]
mod tests;
