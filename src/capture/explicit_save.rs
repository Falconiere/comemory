//! Reference extractor: recover explicit `comemory save` claims from Bash lines.
//!
//! Port of the platform's `candidate-test-support.ts` reference extractor.
//! Engine kinds map to product kinds (`bug → dead-end`, `discovery|note → fact`).

use std::sync::LazyLock;

use regex::Regex;

use crate::capture::claude_code::BashCommand;

/// Extractor id posted on every batch from this path.
pub const EXTRACTOR_ID: &str = "claude-code-explicit-save";

/// Extractor version posted on every batch from this path.
pub const EXTRACTOR_VERSION: u32 = 1;

/// One claim recovered from a save invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedCandidate {
    /// Product kind (`docs/product.md` § 6).
    pub kind: String,
    /// Claim title (≤ 200 chars).
    pub title: String,
    /// Optional body (≤ 4 000 chars).
    pub body: Option<String>,
    /// Optional tags (≤ 16).
    pub tags: Vec<String>,
    /// Extractor confidence (always `1.0` for explicit saves).
    pub confidence: f64,
    /// Transcript timestamp when the save ran.
    pub saved_at: String,
}

static SAVE_INVOCATION: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"comemory(?:\.sh)?\s+save\s").ok());
static DOUBLE_QUOTED: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#""((?:[^"\\]|\\.)*)""#).ok());
static KIND_FLAG: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"--kind\s+([a-z-]+)").ok());
static TAGS_FLAG: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"--tags\s+"((?:[^"\\]|\\.)*)""#).ok());

fn unescape_shell_argument(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek().copied() {
                Some(next @ ('"' | '$' | '`' | '\\')) => {
                    out.push(next);
                    chars.next();
                }
                _ => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn product_kind(engine_kind: &str) -> &'static str {
    match engine_kind {
        "decision" => "decision",
        "convention" => "convention",
        "pattern" => "pattern",
        "bug" => "dead-end",
        // `discovery` and `note` both map to product `fact` (lossy for `note`).
        _ => "fact",
    }
}

/// Recover one claim from a command that invokes `comemory save`, if present.
pub fn extract_from_command(command: &str, saved_at: &str) -> Option<ExtractedCandidate> {
    let save_re = SAVE_INVOCATION.as_ref()?;
    let quoted_re = DOUBLE_QUOTED.as_ref()?;
    let kind_re = KIND_FLAG.as_ref()?;
    let tags_re = TAGS_FLAG.as_ref()?;

    let invocation = save_re.find(command)?;
    let tail = &command[invocation.end()..];

    let tags_match = tags_re.captures(tail);
    let tags = tags_match
        .as_ref()
        .map(|caps| {
            unescape_shell_argument(caps.get(1).map_or("", |m| m.as_str()))
                .split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .take(16)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tags_raw = tags_match
        .as_ref()
        .and_then(|caps| caps.get(1).map(|m| m.as_str()));

    let positionals: Vec<String> = quoted_re
        .captures_iter(tail)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().to_string()))
        .filter(|value| tags_raw != Some(value.as_str()))
        .collect();

    let title_raw = positionals.first()?;
    if title_raw.is_empty() {
        return None;
    }
    let title = unescape_shell_argument(title_raw);
    let title = title.chars().take(200).collect::<String>();
    if title.is_empty() {
        return None;
    }

    let body = positionals.get(1).and_then(|raw| {
        if raw.is_empty() {
            return None;
        }
        let text = unescape_shell_argument(raw);
        let clipped: String = text.chars().take(4_000).collect();
        if clipped.is_empty() {
            None
        } else {
            Some(clipped)
        }
    });

    let engine_kind = kind_re
        .captures(tail)
        .and_then(|caps| caps.get(1).map(|m| m.as_str()))
        .unwrap_or("note");

    Some(ExtractedCandidate {
        kind: product_kind(engine_kind).to_string(),
        title,
        body,
        tags,
        confidence: 1.0,
        saved_at: saved_at.to_string(),
    })
}

/// Every claim the transcript explicitly saved, in transcript order.
pub fn extract_explicit_saves(commands: &[BashCommand]) -> Vec<ExtractedCandidate> {
    commands
        .iter()
        .filter_map(|cmd| extract_from_command(&cmd.command, &cmd.saved_at))
        .collect()
}

#[cfg(test)]
#[path = "tests/explicit_save.rs"]
mod tests;
