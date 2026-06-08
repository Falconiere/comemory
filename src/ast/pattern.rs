//! User-facing ast-grep pattern search.
//!
//! Thin wrapper over `ast_grep_core` so callers can search a source string
//! for arbitrary ast-grep patterns (the same `$VAR` / `$$$ARGS`
//! meta-variable syntax the CLI surfaces) and get back the one-based
//! line + the matched snippet. The full match envelope (env, kind…) is
//! intentionally hidden — extraction is `extractor`'s job.
//!
//! `Lang::Typescript` dispatches to the Tsx grammar so that `.tsx` files
//! with embedded JSX parse without callers needing a separate variant.

use ast_grep_core::tree_sitter::LanguageExt;
use ast_grep_core::{AstGrep, Pattern};
use ast_grep_language::{Go, JavaScript, Python, Rust, Tsx};

use crate::ast::languages::Lang;
use crate::prelude::*;

/// Find every match of `pattern` in `source` for the given language.
///
/// Returns `(line, snippet)` tuples, where `line` is a one-based row.
pub fn find(lang: Lang, source: &str, pattern: &str) -> Result<Vec<(usize, String)>> {
    match lang {
        Lang::Rust => find_with(Rust, source, pattern),
        // Use the Tsx grammar for plain `.ts` and `.tsx` alike — it is a
        // superset that parses JSX-bearing files cleanly without rejecting
        // pure-TypeScript inputs.
        Lang::Typescript => find_with(Tsx, source, pattern),
        Lang::Javascript => find_with(JavaScript, source, pattern),
        Lang::Python => find_with(Python, source, pattern),
        Lang::Go => find_with(Go, source, pattern),
    }
}

fn find_with<L: LanguageExt + Clone>(
    language: L,
    source: &str,
    pattern: &str,
) -> Result<Vec<(usize, String)>> {
    let grep = AstGrep::str(source, language.clone());
    let pat = Pattern::try_new(pattern, language)
        .map_err(|e| Error::Other(format!("ast-grep pattern '{pattern}' failed: {e:?}")))?;
    let mut out = Vec::new();
    for m in grep.root().find_all(&pat) {
        let pos = m.start_pos();
        out.push((pos.line() + 1, m.text().to_string()));
    }
    Ok(out)
}
