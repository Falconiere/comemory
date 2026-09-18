//! Emitters for `comemory search`. The `--json` envelope and its `Row` are
//! the contract both transports serialize, so they live in
//! [`crate::domains::retrieval::search_result`]; this module only writes them
//! out. TTY mode emits one hit per line with a colored score prefix plus a
//! dim path/title line.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use crate::domains::memories::nav::{abs_path, title_of};
use crate::domains::retrieval::rerank::Reranked;
use crate::domains::retrieval::router::TIER_EXPANDED;
use crate::domains::retrieval::scope::ScopeEcho;
use crate::domains::retrieval::search_result::{SearchResult, envelope, source_label};
use crate::output::{json, tty};
use crate::prelude::*;
use crate::store::memory_meta::MemoryMeta;

/// Render `result` to stdout in either JSON or TTY mode. `data_dir` resolves
/// each markdown path to an absolute one. `result.scope` is echoed in the
/// JSON envelope only; the TTY view is unchanged by time scoping.
pub fn emit(result: &SearchResult, json_flag: bool, data_dir: &Path) -> Result<()> {
    let query_id = result.query_id.as_deref();
    if json_flag {
        return json::write(&envelope(
            &result.hits,
            query_id,
            result.meta,
            &result.nav,
            data_dir,
            ScopeEcho::of(&result.scope),
        ));
    }
    write_tty(
        &mut std::io::stdout().lock(),
        &result.hits,
        query_id,
        &result.nav,
        data_dir,
    )
}

/// Render the TTY view of `hits` to `out`. Public so tests can capture the
/// output without going through stdout. Each hit prints a score/source/id
/// line followed by a dim navigation line carrying the markdown path (and
/// title when present). The `query: <qid>` footer semantics live in
/// [`tty::write_query_footer`], shared with `comemory context`.
pub fn write_tty<S: ::std::hash::BuildHasher>(
    out: &mut impl Write,
    hits: &[Reranked],
    query_id: Option<&str>,
    meta: &HashMap<String, MemoryMeta, S>,
    data_dir: &Path,
) -> Result<()> {
    for hit in hits {
        let suffix = match hit.superseded_by.as_deref() {
            Some(id) => format!(" (superseded by {id})"),
            None => String::new(),
        };
        // The expansion tier means the hit was only reachable via a mined
        // query expansion — flag it so users understand the looser match.
        let expanded = if hit.tier == TIER_EXPANDED {
            " [expanded]"
        } else {
            ""
        };
        writeln!(
            out,
            "{}  {}  {}{}{}",
            tty::score(hit.parts.final_score),
            source_label(hit.source),
            hit.memory_id,
            suffix,
            expanded
        )?;
        let path = abs_path(meta.get(&hit.memory_id), data_dir);
        let title = title_of(&hit.body);
        let nav = if title.is_empty() {
            format!("    {path}")
        } else {
            format!("    {title} — {path}")
        };
        writeln!(out, "{}", tty::dim(&nav))?;
    }
    tty::write_query_footer(out, query_id, !hits.is_empty(), tty::FeedbackHint::Memory)
}
