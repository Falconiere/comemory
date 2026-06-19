//! Output helpers for `comemory context`. JSON serializes the
//! [`crate::retrieval::bundle::Bundle`] flattened into an envelope that
//! also carries the optional `query_id` of the retrieval_log row; TTY mode
//! prints a human-readable summary of the matched memories and any code
//! references reached via the graph, plus the same query-id footer as
//! `comemory search` so a context lookup can receive feedback. Code refs
//! arrive prior-ranked from the bundle and are rendered in that order;
//! in JSON, each resolved ref carries its `rank_parts` breakdown
//! ([`crate::retrieval::code_prior::CodePriorParts`], omitted when the
//! ref never resolved to an indexed symbol row).

use std::io::Write as _;

use serde::Serialize;

use crate::output::search::PageMeta;
use crate::output::{json, tty};
use crate::prelude::*;
use crate::retrieval::bundle::Bundle;

/// JSON envelope returned to `--json` callers. The bundle fields stay at the
/// top level (flattened) so existing consumers keep reading `query` /
/// `memories` / `code_refs` / `relations` unchanged; `query_id` is added
/// alongside them, mirroring the `comemory search` envelope. The
/// pagination cursor (`limit` / `offset` / `has_more` / `total`) describes
/// the windowed `memories` list — `total` is the in-window ranked memory
/// count, and the per-memory `code_refs` are left unpaginated (every
/// surfaced memory keeps its full ref set).
#[derive(Serialize)]
pub struct Envelope<'a> {
    /// The assembled context bundle, flattened into the envelope root.
    #[serde(flatten)]
    pub bundle: &'a Bundle<'a>,
    /// Id of the retrieval_log row for this lookup; absent when logging
    /// was off or failed. Feed it back via `comemory feedback <id>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_id: Option<&'a str>,
    /// Requested memory-list page size.
    pub limit: usize,
    /// Number of leading ranked memories skipped.
    pub offset: usize,
    /// Whether more in-window ranked memories exist beyond this page.
    pub has_more: bool,
    /// In-window ranked memory count the page was sliced from; `None` when
    /// not cheaply known.
    pub total: Option<usize>,
}

/// Build the serializable envelope. Public so tests can pin the JSON
/// contract without going through stdout.
pub fn envelope<'a>(
    bundle: &'a Bundle<'a>,
    query_id: Option<&'a str>,
    page: PageMeta,
) -> Envelope<'a> {
    Envelope {
        bundle,
        query_id,
        limit: page.limit,
        offset: page.offset,
        has_more: page.has_more,
        total: page.total,
    }
}

/// Render `bundle` to stdout in either JSON or TTY mode. `query_id` is the
/// retrieval_log id for this lookup (JSON field / TTY footer); `None` skips
/// it. `page` carries the memory-list pagination cursor for the JSON
/// envelope. Footer semantics are shared with `comemory search` via
/// [`tty::write_query_footer`]: the feedback hint only appears when the
/// bundle actually surfaced memories.
pub fn emit<'a>(
    bundle: &'a Bundle<'a>,
    query_id: Option<&'a str>,
    page: PageMeta,
    json_flag: bool,
) -> Result<()> {
    if json_flag {
        return json::write(&envelope(bundle, query_id, page));
    }
    tty::header(&format!("context: {}", bundle.query))?;
    let mut out = std::io::stdout().lock();
    for m in &bundle.memories {
        writeln!(
            out,
            "{}  {}  {}",
            tty::score(m.score),
            m.kind,
            tty::dim(&m.id)
        )?;
    }
    for c in &bundle.code_refs {
        write_code_ref(&mut out, c)?;
    }
    tty::write_query_footer(
        &mut out,
        query_id,
        !bundle.memories.is_empty(),
        tty::FeedbackHint::Memory,
    )
}

/// Render one code ref: the qualified address line (symbol refs keep the
/// `<repo>:<path>:<symbol>` form; file refs drop the trailing colon) followed
/// by a `↳ <path>:<line>  <signature>  [<status>]` detail line. `line` and
/// `signature` are omitted when absent (file refs / unresolved symbols).
fn write_code_ref<W: std::io::Write>(
    out: &mut W,
    c: &crate::retrieval::bundle::CodeRow,
) -> Result<()> {
    if c.symbol.is_empty() {
        writeln!(out, "  {}:{}", c.repo, c.path)?;
    } else {
        writeln!(out, "  {}:{}:{}", c.repo, c.path, c.symbol)?;
    }
    let loc = match c.line {
        Some(n) => format!("{}:{n}", c.path),
        None => c.path.clone(),
    };
    let sig = c.signature.as_deref().unwrap_or("");
    writeln!(
        out,
        "    {} {}  [{}]",
        tty::dim(&format!("↳ {loc}")),
        sig,
        c.status
    )?;
    Ok(())
}
