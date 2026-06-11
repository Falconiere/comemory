//! Output helpers for `comemory context`. JSON serializes the
//! [`crate::retrieval::bundle::Bundle`] flattened into an envelope that
//! also carries the optional `query_id` of the retrieval_log row; TTY mode
//! prints a human-readable summary of the matched memories and any code
//! references reached via the graph, plus the same query-id footer as
//! `comemory search` so a context lookup can receive feedback.

use std::io::Write as _;

use serde::Serialize;

use crate::output::{json, tty};
use crate::prelude::*;
use crate::retrieval::bundle::Bundle;

/// JSON envelope returned to `--json` callers. The bundle fields stay at the
/// top level (flattened) so existing consumers keep reading `query` /
/// `memories` / `code_refs` / `relations` unchanged; `query_id` is added
/// alongside them, mirroring the `comemory search` envelope.
#[derive(Serialize)]
pub struct Envelope<'a> {
    /// The assembled context bundle, flattened into the envelope root.
    #[serde(flatten)]
    pub bundle: &'a Bundle<'a>,
    /// Id of the retrieval_log row for this lookup; absent when logging
    /// was off or failed. Feed it back via `comemory feedback <id>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_id: Option<&'a str>,
}

/// Build the serializable envelope. Public so tests can pin the JSON
/// contract without going through stdout.
pub fn envelope<'a>(bundle: &'a Bundle<'a>, query_id: Option<&'a str>) -> Envelope<'a> {
    Envelope { bundle, query_id }
}

/// Render `bundle` to stdout in either JSON or TTY mode. `query_id` is the
/// retrieval_log id for this lookup (JSON field / TTY footer); `None` skips
/// it. Footer semantics are shared with `comemory search` via
/// [`tty::write_query_footer`]: the feedback hint only appears when the
/// bundle actually surfaced memories.
pub fn emit<'a>(bundle: &'a Bundle<'a>, query_id: Option<&'a str>, json_flag: bool) -> Result<()> {
    if json_flag {
        return json::write(&envelope(bundle, query_id));
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
        writeln!(out, "  {}:{}:{}", c.repo, c.path, c.symbol)?;
    }
    tty::write_query_footer(&mut out, query_id, !bundle.memories.is_empty())
}
