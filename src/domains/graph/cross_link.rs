//! Extract `<repo>:<path>[:<symbol>]` references from a memory body.
//!
//! [`crate::domains::memories::mirror`] scans every memory body with
//! [`extract_refs`](crate::domains::graph::cross_link::extract_refs) before the
//! mirror write; `store::memory_row` turns the result into `ReferencesFile` /
//! `ReferencesSymbol` edges. This module extracts and never writes. The parser
//! is intentionally simple: a single regex match per token, with deduplication
//! so a body that mentions the same file twice produces a single edge.

use regex::Regex;

/// Code-layer references harvested from a memory body.
///
/// The vectors are de-duplicated and preserve first-mention order so the
/// caller can reproduce a stable edge insertion sequence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Refs {
    /// Qualified file names in the form `<repo>:<path>`.
    pub files: Vec<String>,
    /// Qualified symbol names in the form `<repo>:<path>:<symbol>`.
    pub symbols: Vec<String>,
}

/// Compiled once per process. The literal is static and well-formed, so
/// `Regex::new` cannot fail in practice; `.ok()` keeps the type panic-free
/// (no `expect` / `unwrap` calls) and [`extract_refs`] treats an (impossible)
/// `None` as "no references".
static REF_RE: std::sync::LazyLock<Option<Regex>> = std::sync::LazyLock::new(|| {
    Regex::new(r"\b([a-z0-9_-]+):([A-Za-z0-9_./\-]+\.[a-zA-Z]+)(?::([A-Za-z_][A-Za-z0-9_]*))?\b")
        .ok()
});

/// Scan `body` for `<repo>:<path>` and `<repo>:<path>:<symbol>` tokens.
///
/// Every match yields a file ref; matches that include the optional symbol
/// suffix also yield a symbol ref. Results are de-duplicated while preserving
/// first-mention order.
///
/// URL-like patterns (`https://github.com/foo/bar.rs`,
/// `git@github.com:foo/bar.rs`) and path expressions behind a bare scheme
/// (`file:/tmp/check.db`, `sqlite:./data.db`, `file:../../local.db`) are
/// rejected so cross-link extraction doesn't invent bogus `<repo>:<path>`
/// candidates from prose that just happens to contain a link, an scp-style
/// git URL, or a database location.
///
/// The filter is post-extraction — Rust's `regex` crate has no lookbehind, so
/// after a match we re-inspect the non-whitespace prefix immediately preceding
/// it. Either `://` or `@` in that prefix classifies the surrounding token as
/// a URL, and a captured path that [`is_path_expression`] classifies the token
/// as a path rather than a repo-relative citation; both drop the match.
pub fn extract_refs(body: &str) -> Refs {
    let Some(re) = REF_RE.as_ref() else {
        return Refs::default();
    };
    let bytes = body.as_bytes();
    let mut refs = Refs::default();
    for cap in re.captures_iter(body) {
        let Some(whole) = cap.get(0) else { continue };
        let start = whole.start();
        let Some(repo) = cap.get(1) else { continue };
        let Some(path) = cap.get(2) else { continue };
        // `https://github.com/foo.rs` → captured path begins with `//`;
        // `file:/tmp/check.db` / `sqlite:./data.db` → `/` or a dot segment.
        if is_path_expression(path.as_str()) {
            continue;
        }
        // Walk back to the start of the contiguous non-whitespace run that
        // contains the match. If that prefix has a URL hallmark — `://` for
        // schemed URLs, `@` for scp-style git remotes — the match is part of
        // a URL, not a comemory ref.
        let prefix_start = bytes[..start]
            .iter()
            .rposition(u8::is_ascii_whitespace)
            .map_or(0, |i| i + 1);
        let prefix = &bytes[prefix_start..start];
        if prefix.windows(3).any(|w| w == b"://") || prefix.contains(&b'@') {
            continue;
        }
        let file_q = format!("{}:{}", repo.as_str(), path.as_str());
        if !refs.files.contains(&file_q) {
            refs.files.push(file_q.clone());
        }
        if let Some(sym) = cap.get(3) {
            let sym_q = format!("{}:{}", file_q, sym.as_str());
            if !refs.symbols.contains(&sym_q) {
                refs.symbols.push(sym_q);
            }
        }
    }
    refs
}

/// True when a captured `<path>` is a filesystem or URL path expression
/// rather than a repo-relative citation: absolute (`/tmp/check.db`, and the
/// `//host/…` of a schemed URL) or dot-relative (`./data.db`,
/// `../../local.db`). A `<repo>:<path>` reference is always relative to the
/// repo root, so none of these shapes can be one — they are the `file:` /
/// `sqlite:` URLs prose records a database location with, which the
/// `://`/`@` prefix guard alone let through as a pseudo-repo named after
/// the scheme (issue #153).
fn is_path_expression(path: &str) -> bool {
    path.starts_with('/') || path.starts_with("./") || path.starts_with("../")
}

#[cfg(test)]
#[path = "tests/cross_link.rs"]
mod tests;
