//! Symbol extraction via `ast-grep-core` patterns.
//!
//! Given a `Lang` and a source string, returns every top-level function /
//! class / struct / enum / trait definition together with the snippet text
//! and one-based start line. The set of patterns per language is small and
//! intentionally generic — we are not building a full language server here,
//! just a corpus of indexable snippets for the code-side embedding store.
//!
//! `Lang::Typescript` dispatches to the Tsx grammar so JSX-bearing `.tsx`
//! files extract just as cleanly as plain `.ts`.

use ast_grep_core::tree_sitter::{LanguageExt, StrDoc};
use ast_grep_core::{AstGrep, NodeMatch, Pattern};
use ast_grep_language::{Go, JavaScript, Python, Rust, Tsx};

use crate::ast::chunk::{self, Chunk, CHUNK_LINE_BUDGET};
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
    /// cAST chunks for symbols whose span exceeds
    /// [`CHUNK_LINE_BUDGET`] lines; empty means the symbol is stored
    /// whole (unchunked).
    pub chunks: Vec<Chunk>,
}

/// Run every per-kind pattern for `lang` against `source` and return the
/// flat list of symbols. The order is pattern-first, source-second.
pub fn extract(lang: Lang, source: &str) -> Result<Vec<ExtractedSymbol>> {
    match lang {
        Lang::Rust => extract_with(Rust, lang, source, rust_patterns()),
        // Tsx is a superset of the plain TypeScript grammar — it parses
        // JSX-bearing source as well as pure TS, so we route both `.ts` and
        // `.tsx` through it.
        Lang::Typescript => extract_with(Tsx, lang, source, ts_patterns()),
        Lang::Javascript => extract_with(JavaScript, lang, source, js_patterns()),
        Lang::Python => extract_with(Python, lang, source, python_patterns()),
        Lang::Go => extract_with(Go, lang, source, go_patterns()),
    }
}

fn rust_patterns() -> &'static [(&'static str, &'static str)] {
    // Two axes are spelled out explicitly because ast-grep patterns match
    // Rust definitions strictly (a `fn $NAME(...)` pattern does NOT match
    // `pub fn` or `async fn` — probed empirically):
    //   * return clause: `-> Ret` present vs absent;
    //   * modifiers: bare / `pub` / `async` / `pub async`.
    // The `pub` patterns also cover `pub(crate)` / `pub(super)` — ast-grep
    // treats the visibility node loosely, same as the `pub use` import
    // patterns in `crate::graph::imports`. Known gap, accepted to keep the
    // table small: `const fn` / `unsafe fn` items (any visibility) are not
    // matched.
    &[
        ("function", "fn $NAME($$$ARGS) -> $RET { $$$BODY }"),
        ("function", "fn $NAME($$$ARGS) { $$$BODY }"),
        ("function", "pub fn $NAME($$$ARGS) -> $RET { $$$BODY }"),
        ("function", "pub fn $NAME($$$ARGS) { $$$BODY }"),
        ("function", "async fn $NAME($$$ARGS) -> $RET { $$$BODY }"),
        ("function", "async fn $NAME($$$ARGS) { $$$BODY }"),
        (
            "function",
            "pub async fn $NAME($$$ARGS) -> $RET { $$$BODY }",
        ),
        ("function", "pub async fn $NAME($$$ARGS) { $$$BODY }"),
        ("struct", "struct $NAME { $$$BODY }"),
        ("struct", "pub struct $NAME { $$$BODY }"),
        ("enum", "enum $NAME { $$$BODY }"),
        ("enum", "pub enum $NAME { $$$BODY }"),
        ("trait", "trait $NAME { $$$BODY }"),
        ("trait", "pub trait $NAME { $$$BODY }"),
    ]
}

fn ts_patterns() -> &'static [(&'static str, &'static str)] {
    // TypeScript functions may carry a return annotation (`: number`)
    // between the arg list and the body; JavaScript skips it. List both
    // shapes so we recover the function name in either case.
    //
    // `export` / `export default` / `async` prefixes need no extra
    // patterns: the wrapped declaration is a child node of the export
    // statement and `find_all` descends into it (probed empirically).
    // `abstract class` is a distinct node kind, so it gets its own row —
    // it must stay LAST so `js_patterns` can reuse this table minus it.
    &[
        ("function", "function $NAME($$$ARGS): $RET { $$$BODY }"),
        ("function", "function $NAME($$$ARGS) { $$$BODY }"),
        ("class", "class $NAME { $$$BODY }"),
        ("class", "abstract class $NAME { $$$BODY }"),
    ]
}

fn js_patterns() -> &'static [(&'static str, &'static str)] {
    // JavaScript shares the TS table except `abstract class`, which the
    // JS grammar rejects at pattern-compile time (no abstract classes).
    let all = ts_patterns();
    &all[..all.len() - 1]
}

fn python_patterns() -> &'static [(&'static str, &'static str)] {
    // Decorated defs/classes and `async def` need no extra patterns — the
    // wrapped definition is a child of the decorated node and `find_all`
    // descends into it (probed empirically). A base-class list however
    // changes the node shape, so `class Foo(Base):` gets its own row.
    &[
        ("function", "def $NAME($$$ARGS): $$$BODY"),
        ("class", "class $NAME: $$$BODY"),
        ("class", "class $NAME($$$BASES): $$$BODY"),
    ]
}

fn go_patterns() -> &'static [(&'static str, &'static str)] {
    // Go functions can be free-standing (`func Foo(...) { ... }`) or
    // method-receiver bound (`func (r R) Foo(...) { ... }`). Each shape is
    // listed with and without a return-type clause so the `$NAME` binding
    // is recovered whether or not the function returns a value.
    &[
        ("function", "func $NAME($$$ARGS) $RET { $$$BODY }"),
        ("function", "func $NAME($$$ARGS) { $$$BODY }"),
        ("function", "func ($$$RECV) $NAME($$$ARGS) $RET { $$$BODY }"),
        ("function", "func ($$$RECV) $NAME($$$ARGS) { $$$BODY }"),
    ]
}

/// Compile each `(tag, pattern)` pair for `language` and invoke `on_match`
/// with the tag and every node the pattern matches in `source`. Shared by
/// symbol extraction here and import extraction in `crate::graph::imports`.
pub(crate) fn for_each_match<L, F>(
    language: L,
    source: &str,
    patterns: &[(&str, &str)],
    mut on_match: F,
) -> Result<()>
where
    L: LanguageExt + Clone,
    F: FnMut(&str, &NodeMatch<StrDoc<L>>),
{
    let grep = AstGrep::str(source, language.clone());
    let root = grep.root();
    for (tag, pat) in patterns {
        let pattern = Pattern::try_new(pat, language.clone())
            .map_err(|e| Error::Other(format!("ast-grep pattern '{pat}' failed: {e:?}")))?;
        for matched in root.find_all(&pattern) {
            on_match(tag, &matched);
        }
    }
    Ok(())
}

fn extract_with<L: LanguageExt + Clone>(
    language: L,
    lang: Lang,
    source: &str,
    patterns: &[(&str, &str)],
) -> Result<Vec<ExtractedSymbol>> {
    let mut out = Vec::new();
    for_each_match(language, source, patterns, |kind, m| {
        let Some(name_node) = m.get_env().get_match("NAME") else {
            return;
        };
        let name = name_node.text().to_string();
        if name.is_empty() {
            return;
        }
        let node = m.get_node();
        let (line_start, line_end) = chunk::line_span(node);
        // Oversized symbols are additionally split into cAST chunks so
        // downstream FTS/embedding rows stay within the line budget.
        let chunks = if line_end - line_start + 1 > CHUNK_LINE_BUDGET {
            chunk::chunk_node(node, source)
        } else {
            Vec::new()
        };
        out.push(ExtractedSymbol {
            name,
            kind: kind.to_string(),
            language: lang.as_str().to_string(),
            snippet: m.text().to_string(),
            line: line_start,
            chunks,
        });
    })?;
    Ok(out)
}
