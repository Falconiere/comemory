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
