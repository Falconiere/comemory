//! Emitters for `comemory context`. The flattened-bundle envelope is the
//! contract both transports serialize, so it lives in
//! [`crate::domains::retrieval::context_result`]; this module only writes it
//! out. TTY mode prints a human-readable summary of the matched memories and
//! any code references reached via the graph, plus the same query-id footer
//! as `comemory search` so a context lookup can receive feedback. Code refs
//! arrive prior-ranked from the bundle and are rendered in that order;
//! in JSON, each resolved ref carries its `rank_parts` breakdown
//! ([`crate::domains::retrieval::code_prior::CodePriorParts`], omitted when the
//! ref never resolved to an indexed symbol row).

use std::io::Write as _;

use crate::domains::retrieval::context_result::{ContextResult, envelope};
use crate::domains::retrieval::scope::ScopeEcho;
use crate::output::{json, tty};
use crate::prelude::*;

/// Render `result` to stdout in either JSON or TTY mode. Footer semantics
/// are shared with `comemory search` via [`tty::write_query_footer`]: the
/// feedback hint only appears when the bundle actually surfaced memories.
/// The time scope is echoed in the JSON envelope only; the TTY view is
/// unchanged by time scoping.
pub fn emit(result: &ContextResult, json_flag: bool) -> Result<()> {
    let ContextResult {
        bundle,
        query_id,
        meta,
        scope,
    } = result;
    let query_id = query_id.as_deref();
    if json_flag {
        return json::write(&envelope(bundle, query_id, *meta, ScopeEcho::of(scope)));
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
    c: &crate::domains::retrieval::bundle::CodeRow,
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

#[cfg(test)]
#[path = "tests/context.rs"]
mod tests;
