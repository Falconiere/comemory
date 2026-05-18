//! User-facing ast-grep pattern search.
//!
//! Thin wrapper over `ast_grep_core` so callers can search a source string
//! for arbitrary ast-grep patterns (the same `$VAR` / `$$$ARGS`
//! meta-variable syntax the CLI surfaces) and get back the one-based
//! line + the matched snippet. The full match envelope (env, kind…) is
//! intentionally hidden — extraction is `extractor`'s job.

use ast_grep_core::tree_sitter::LanguageExt;
use ast_grep_core::{AstGrep, Pattern};
use ast_grep_language::{JavaScript, Python, Rust, TypeScript};

use crate::ast::languages::Lang;
use crate::prelude::*;

/// Find every match of `pattern` in `source` for the given language.
///
/// Returns `(line, snippet)` tuples, where `line` is a one-based row.
pub fn find(lang: Lang, source: &str, pattern: &str) -> Result<Vec<(usize, String)>> {
    match lang {
        Lang::Rust => find_with(Rust, source, pattern),
        Lang::TypeScript => find_with(TypeScript, source, pattern),
        Lang::JavaScript => find_with(JavaScript, source, pattern),
        Lang::Python => find_with(Python, source, pattern),
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
