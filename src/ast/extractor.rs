//! Symbol extraction via `ast-grep-core` patterns.
//!
//! Given a `Lang` and a source string, returns every top-level function /
//! class / struct / enum / trait definition together with the snippet text
//! and one-based start line. The set of patterns per language is small and
//! intentionally generic — we are not building a full language server here,
//! just a corpus of indexable snippets for the code-side embedding store.

use ast_grep_core::tree_sitter::LanguageExt;
use ast_grep_core::{AstGrep, Pattern};
use ast_grep_language::{JavaScript, Python, Rust, Tsx, TypeScript};

use crate::ast::languages::Lang;
use crate::prelude::*;

/// One extracted symbol from a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedSymbol {
    /// Identifier name (e.g. function name, struct name).
    pub name: String,
    /// Symbol kind: `"function"`, `"struct"`, `"enum"`, `"trait"`, `"class"`.
    pub kind: String,
    /// Lowercased language name from `Lang::as_str`.
    pub language: String,
    /// Source text of the entire match (the whole definition).
    pub snippet: String,
    /// One-based line number of the start of the match.
    pub line: usize,
}

/// Run every per-kind pattern for `lang` against `source` and return the
/// flat list of symbols. The order is pattern-first, source-second.
pub fn extract(lang: Lang, source: &str) -> Result<Vec<ExtractedSymbol>> {
    match lang {
        Lang::Rust => extract_with(Rust, lang, source, rust_patterns()),
        Lang::TypeScript => extract_with(TypeScript, lang, source, ts_js_patterns()),
        // `Tsx` uses the same function/class patterns as TypeScript — the
        // difference is the grammar, not the surface shape we extract.
        Lang::Tsx => extract_with(Tsx, lang, source, ts_js_patterns()),
        Lang::JavaScript => extract_with(JavaScript, lang, source, ts_js_patterns()),
        Lang::Python => extract_with(Python, lang, source, python_patterns()),
    }
}

fn rust_patterns() -> &'static [(&'static str, &'static str)] {
    // Functions in Rust may have a `-> Ret` return-type clause between
    // the arg list and the body — the explicit return arrow is the only
    // pattern that ast-grep matches against `fn add(...) -> i32 { ... }`.
    // We list both variants so plain `fn foo() { ... }` is still picked up.
    &[
        ("function", "fn $NAME($$$ARGS) -> $RET { $$$BODY }"),
        ("function", "fn $NAME($$$ARGS) { $$$BODY }"),
        ("struct", "struct $NAME { $$$BODY }"),
        ("enum", "enum $NAME { $$$BODY }"),
        ("trait", "trait $NAME { $$$BODY }"),
    ]
}

fn ts_js_patterns() -> &'static [(&'static str, &'static str)] {
    // TypeScript functions may carry a return annotation (`: number`)
    // between the arg list and the body; JavaScript skips it. List both
    // shapes so we recover the function name in either case.
    &[
        ("function", "function $NAME($$$ARGS): $RET { $$$BODY }"),
        ("function", "function $NAME($$$ARGS) { $$$BODY }"),
        ("class", "class $NAME { $$$BODY }"),
    ]
}

fn python_patterns() -> &'static [(&'static str, &'static str)] {
    &[
        ("function", "def $NAME($$$ARGS): $$$BODY"),
        ("class", "class $NAME: $$$BODY"),
    ]
}

fn extract_with<L: LanguageExt + Clone>(
    language: L,
    lang: Lang,
    source: &str,
    patterns: &[(&str, &str)],
) -> Result<Vec<ExtractedSymbol>> {
    let grep = AstGrep::str(source, language.clone());
    let root = grep.root();
    let mut out = Vec::new();
    for (kind, pat) in patterns {
        let pattern = Pattern::try_new(pat, language.clone())
            .map_err(|e| Error::Other(format!("ast-grep pattern '{pat}' failed: {e:?}")))?;
        for m in root.find_all(&pattern) {
            let Some(name_node) = m.get_env().get_match("NAME") else {
                continue;
            };
            let name = name_node.text().to_string();
            if name.is_empty() {
                continue;
            }
            let pos = m.start_pos();
            out.push(ExtractedSymbol {
                name,
                kind: (*kind).to_string(),
                language: lang.as_str().to_string(),
                snippet: m.text().to_string(),
                line: pos.line() + 1,
            });
        }
    }
    Ok(out)
}
