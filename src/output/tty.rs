//! ANSI-colored renderers for human-readable CLI output. Each helper returns
//! a `String` carrying the appropriate `owo-colors` escape sequences so call
//! sites can compose them into a `writeln!`. `header` writes directly to
//! stdout because it is meant to stand alone above a section.

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
