//! Turning indexed file paths into component keys, ids, and seed summaries.
//!
//! The clustering axis is the directory tree, truncated to a depth: it is the
//! axis a repository is actually organised on, it is stable across indexing
//! runs (so a re-scaffold produces the same ids), and it needs no heuristics a
//! reader would have to trust.

use std::path::Path;

use crate::domains::architecture::model::MAX_SUMMARY;

/// The component key for `path`: its leading `depth` directory segments, or
/// its own parent directory when it is shallower than that. A file at the
/// repository root is its own key, since it has no directory to belong to.
pub fn key_for(path: &str, depth: usize) -> String {
    let segments: Vec<&str> = path.split('/').collect();
    let dirs = segments.len().saturating_sub(1);
    let take = depth.min(dirs);
    if take == 0 {
        return path.to_string();
    }
    segments[..take].join("/")
}

/// Whether `prefix` covers `path`: the same path, or a parent directory of
/// it. A bare textual `starts_with` would let `src/co` claim `src/code/a.rs`,
/// so the match must land on a path boundary. Shared by member validation and
/// the drift check, which ask the same question of different inputs.
pub fn covers(prefix: &str, path: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|r| r.starts_with('/'))
}

/// A renderer-safe id derived from a component key: every run of characters
/// outside `[A-Za-z0-9]` collapses to one `_`, and a key that would not start
/// with a letter is prefixed. Matches `^[A-Za-z][A-Za-z0-9_]{0,63}$`, the
/// shape [`validate`](super::validate) enforces.
pub fn ident(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut pending_sep = false;
    for c in key.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_sep && !out.is_empty() {
                out.push('_');
            }
            pending_sep = false;
            out.push(c);
        } else {
            pending_sep = true;
        }
    }
    if !out.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        out.insert_str(0, "c_");
    }
    out.truncate(64);
    out.trim_end_matches('_').to_string()
}

/// The first sentence of `<root>/<key>/README.md`, when that file exists — the
/// cheapest honest seed for a component summary, since a folder README in this
/// codebase opens with what the folder is for. Returns `None` for a missing,
/// unreadable, or heading-only README.
pub fn summary_from_readme(root: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(root.join(key).join("README.md")).ok()?;
    let paragraph = text
        .lines()
        .map(str::trim)
        .skip_while(|l| l.is_empty() || l.starts_with('#'))
        .take_while(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let sentence = first_sentence(&paragraph);
    (!sentence.is_empty()).then_some(sentence)
}

/// The first sentence of `paragraph`, stripped of markdown emphasis and
/// capped at [`MAX_SUMMARY`] characters on a character boundary.
fn first_sentence(paragraph: &str) -> String {
    let plain: String = paragraph
        .chars()
        .filter(|c| !matches!(c, '*' | '`' | '[' | ']'))
        .collect();
    let end = plain.find(". ").map_or(plain.len(), |i| i + 1);
    plain[..end]
        .chars()
        .take(MAX_SUMMARY)
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
#[path = "tests/cluster.rs"]
mod tests;
