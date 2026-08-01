//! `--only` domain-scope resolution, plus the interim `--only document`
//! search path for `comemory search`.
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
use rusqlite::Connection;
use serde::Serialize;

use crate::config::Config;
use crate::output::page::Page;
use crate::output::{json, tty};
use crate::prelude::*;
use crate::retrieval::doc_route::{self, DocHit};
use crate::retrieval::pipeline::{self, PageWindow};
use crate::retrieval::scope::{Domain, Domains, Filters};

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

/// Resolve `--only` (raw clap values) plus `--kind` into the [`Domains`]
/// mask a search run should use. `--only` unset: `--kind` narrows the
/// default to memory-only, else the full default stands. `--only` set:
/// parsed verbatim, rejected as a usage error if it excludes memory
/// while `--kind` is set (nothing to apply the filter to), or if it
/// includes [`Domain::Code`] — no leg here searches code yet, so it
/// would otherwise silently drop code results instead of finding any.
pub fn resolve_domains(only: &[OnlyDomain], kind: Option<&str>) -> Result<Domains> {
    if only.is_empty() {
        return Ok(if kind.is_some() {
            Domains::memory_only()
        } else {
            Domains::all()
        });
    }
    let domains = Domains::of(&only.iter().map(|d| Domain::from(*d)).collect::<Vec<_>>());
    if let Some(k) = kind
        && !domains.contains(Domain::Memory)
    {
        return Err(Error::Usage(format!(
            "--kind {k} requires memory in --only (got: {})",
            only_label(only)
        )));
    }
    if domains.contains(Domain::Code) {
        return Err(Error::Usage(format!(
            "code domain joins unified search in a later release; use `comemory search-code` \
             instead (got: --only {})",
            only_label(only)
        )));
    }
    Ok(domains)
}

/// Render `only` back as its comma-separated flag values, for the
/// contradiction error in [`resolve_domains`].
fn only_label(only: &[OnlyDomain]) -> String {
    only.iter()
        .filter_map(|d| d.to_possible_value())
        .map(|v| v.get_name().to_string())
        .collect::<Vec<_>>()
        .join(",")
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
    DocRow {
        domain: "document",
        document_id: &h.document_id,
        title: &h.title,
        snippet: &h.snippet,
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
        writeln!(
            out,
            "{}  {} ({}:{}-{})",
            hit.document_id, hit.title, hit.path, hit.line_range.0, hit.line_range.1
        )?;
        writeln!(out, "{}", tty::dim(&hit.snippet))?;
    }
    tty::write_page_footer(out, page.items.len(), page.offset, page.total)
}
