//! Derive a filename-safe slug from a memory body.
//!
//! Picks the first non-empty line, lowercases ASCII alphanumerics, collapses
//! every other run into a single `-`, trims leading/trailing dashes, and caps
//! length. Falls back to `"untitled"` when the input is whitespace-only.

const MAX_SLUG_LEN: usize = 60;

/// Build a filename-safe slug from the first meaningful line of `body`.
pub fn slug_from_body(body: &str) -> String {
    let first = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out = String::with_capacity(MAX_SLUG_LEN);
    let mut prev_dash = false;
    for c in first.chars() {
        let mapped = if c.is_ascii_alphanumeric() {
            c.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' {
            if !prev_dash && !out.is_empty() {
                out.push('-');
                prev_dash = true;
            }
        } else {
            out.push(mapped);
            prev_dash = false;
        }
        if out.len() >= MAX_SLUG_LEN {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "untitled".into()
    } else {
        trimmed
    }
}
