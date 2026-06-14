//! Extract `<repo>:<path>[:<symbol>]` references from a memory body.
//!
//! The save pipeline scans every memory body with [`extract_refs`] and creates
//! `ReferencesFile` / `ReferencesSymbol` edges to the code-layer nodes. The
//! parser is intentionally simple: a single regex match per token, with
//! deduplication so a body that mentions the same file twice produces a single
//! edge.

use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::Connection;

use crate::graph::edges::{self, EdgeKey};
use crate::prelude::*;

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
static REF_RE: Lazy<Option<Regex>> = Lazy::new(|| {
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
/// `git@github.com:foo/bar.rs`) are rejected so cross-link extraction doesn't
/// invent bogus `<repo>:<path>` candidates from prose that just happens to
/// contain a link or scp-style git URL.
///
/// The filter is post-extraction — Rust's `regex` crate has no lookbehind, so
/// after a match we re-inspect the non-whitespace prefix immediately preceding
/// it. Any of `://`, `@`, or a `//` prefix on the captured path is enough to
/// classify the surrounding token as a URL, in which case the match is dropped.
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
        // `https://github.com/foo.rs` → captured path begins with `//`.
        if path.as_str().starts_with("//") {
            continue;
        }
        // Walk back to the start of the contiguous non-whitespace run that
        // contains the match. If that prefix has a URL hallmark — `://` for
        // schemed URLs, `@` for scp-style git remotes — the match is part of
        // a URL, not a comemory ref.
        let prefix_start = bytes[..start]
            .iter()
            .rposition(|b| b.is_ascii_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
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

/// Walk `body`, extract every `<repo>:<path>[:<symbol>]` reference, and
/// insert `references_file` / `references_symbol` edges into the v0.2
/// `edges` table. Node addressing matches `src/store/sql/0002_v2_tables.sql`:
/// `file:<repo>:<path>` and `symbol:<repo>:<path>:<symbol>`. Replaces the
/// v0.1 kuzu writer; the file/symbol nodes themselves are populated later
/// by `comemory index-code`.
pub fn extract_and_emit(conn: &Connection, memory_id: &str, body: &str) -> Result<()> {
    let refs = extract_refs(body);
    for file_q in &refs.files {
        edges::insert(
            conn,
            EdgeKey {
                src_kind: "memory",
                src_id: memory_id,
                dst_kind: "file",
                dst_id: file_q,
                rel: "references_file",
            },
        )?;
    }
    for sym_q in &refs.symbols {
        edges::insert(
            conn,
            EdgeKey {
                src_kind: "memory",
                src_id: memory_id,
                dst_kind: "symbol",
                dst_id: sym_q,
                rel: "references_symbol",
            },
        )?;
    }
    Ok(())
}
