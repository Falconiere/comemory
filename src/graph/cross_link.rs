//! Extract `<repo>:<path>[:<symbol>]` references from a memory body.
//!
//! The save pipeline scans every memory body with [`extract_refs`] and creates
//! `ReferencesFile` / `ReferencesSymbol` edges to the code-layer nodes. The
//! parser is intentionally simple: a single regex match per token, with
//! deduplication so a body that mentions the same file twice produces a single
//! edge.

use once_cell::sync::Lazy;
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

/// Compiled once per process. The literal is static, the pattern is well-formed,
/// so the only failure mode for `Regex::new` would be a programmer error caught
/// by the test suite — hence `expect` with an explanatory message.
static REF_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b([a-z0-9_-]+):([A-Za-z0-9_./\-]+\.[a-zA-Z]+)(?::([A-Za-z_][A-Za-z0-9_]*))?\b")
        .expect("cross-link reference regex must compile")
});

/// Scan `body` for `<repo>:<path>` and `<repo>:<path>:<symbol>` tokens.
///
/// Every match yields a file ref; matches that include the optional symbol
/// suffix also yield a symbol ref. Results are de-duplicated while preserving
/// first-mention order.
pub fn extract_refs(body: &str) -> Refs {
    let mut refs = Refs::default();
    for cap in REF_RE.captures_iter(body) {
        let Some(repo) = cap.get(1) else { continue };
        let Some(path) = cap.get(2) else { continue };
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
