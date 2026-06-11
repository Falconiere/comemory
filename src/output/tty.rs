//! ANSI-colored renderers for human-readable CLI output. Each helper returns
//! a `String` carrying the appropriate `owo-colors` escape sequences so call
//! sites can compose them into a `writeln!`. Two helpers write directly
//! instead: `header` to stdout because it stands alone above a section, and
//! `warning` to stderr so advisory lines stay out of pipeable stdout.

use std::io::Write as _;

use owo_colors::OwoColorize;

use crate::prelude::*;

/// Render `s` as a bold cyan section header on stdout, followed by a newline.
pub fn header(s: &str) -> Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", s.bold().cyan())?;
    Ok(())
}

/// Render `msg` as a yellow `warning: <msg>` line on stderr, followed by a
/// newline. Stderr keeps advisory output out of pipelines that consume
/// stdout (ids, JSON envelopes).
pub fn warning(msg: &str) -> Result<()> {
    let mut err = std::io::stderr().lock();
    writeln!(err, "{}", format!("warning: {msg}").yellow())?;
    Ok(())
}

/// Format a similarity score (`0.0..=1.0`) as a yellow `0.xxx` string with
/// three fractional digits. Returned `String` is meant to be embedded inside
/// a larger `writeln!`.
pub fn score(v: f32) -> String {
    format!("{:.3}", v).yellow().to_string()
}

/// Wrap `s` in the dim ANSI style. Returned `String` is meant to be embedded
/// inside a larger `writeln!`.
pub fn dim(s: &str) -> String {
    s.dimmed().to_string()
}

/// Which `comemory feedback` flag family the query footer's hint should
/// reference. Memory results are fed back via `--used <ids>` (8-hex memory
/// ids); code results via `--used-code <ids>` (integer `code_symbols` ids).
/// One enum + one footer writer keeps the two hints from drifting apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackHint {
    /// Memory-id verdicts: `comemory feedback <qid> --used <ids>`.
    Memory,
    /// Code-symbol verdicts: `comemory feedback <qid> --used-code <ids>`.
    Code,
}

impl FeedbackHint {
    /// The feedback flag this flavor's hint names.
    fn flag(self) -> &'static str {
        match self {
            FeedbackHint::Memory => "--used",
            FeedbackHint::Code => "--used-code",
        }
    }
}

/// Write the shared `query: <qid>` TTY footer used by `comemory search`,
/// `comemory search-code`, and `comemory context`. The footer is printed
/// whenever a query id exists — zero-hit queries are still logged for
/// reformulation mining — but the feedback hint is appended only when
/// `has_hits`, since with no hits there is nothing to mark used. `hint`
/// picks which feedback flag family the appended hint references.
pub fn write_query_footer(
    out: &mut impl std::io::Write,
    query_id: Option<&str>,
    has_hits: bool,
    hint: FeedbackHint,
) -> Result<()> {
    if let Some(qid) = query_id {
        let suffix = if has_hits {
            format!(
                "  (feedback: comemory feedback {qid} {} <ids>)",
                hint.flag()
            )
        } else {
            String::new()
        };
        writeln!(out, "query: {qid}{suffix}")?;
    }
    Ok(())
}
