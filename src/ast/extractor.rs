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

use std::collections::HashSet;
use std::sync::OnceLock;

use ast_grep_core::tree_sitter::{LanguageExt, StrDoc};
use ast_grep_core::{AstGrep, NodeMatch, Pattern};
use ast_grep_language::{Go, JavaScript, Python, Rust, Tsx};

use crate::ast::chunk::{self, CHUNK_LINE_BUDGET, Chunk};
use crate::ast::languages::Lang;
use crate::ast::pattern_cache::{self, CompiledPatterns};
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
    /// One-based line number of the (inclusive) end of the match, taken
    /// from the same tree-sitter span as [`ExtractedSymbol::line`] — the
    /// authoritative value persistence must store, rather than re-deriving
    /// it from the snippet's line count (which can disagree at a
    /// column-0 end edge).
    pub line_end: usize,
    /// cAST chunks for symbols whose span exceeds
    /// [`CHUNK_LINE_BUDGET`] lines; empty means the symbol is stored
    /// whole (unchunked).
    pub chunks: Vec<Chunk>,
}

/// Run every per-kind pattern for `lang` against `source` and return the
/// flat list of symbols. The order is pattern-first, source-second.
///
/// Each language's pattern table is compiled exactly once into a process
/// `static` (see [`pattern_cache`]) and reused across every file, so a repo
/// walk pays the compile cost once per language rather than once per file.
pub fn extract(lang: Lang, source: &str) -> Result<Vec<ExtractedSymbol>> {
    match lang {
        Lang::Rust => extract_with(Rust, lang, source, rust_compiled()?),
        // Tsx is a superset of the plain TypeScript grammar — it parses
        // JSX-bearing source as well as pure TS, so we route both `.ts` and
        // `.tsx` through it.
        Lang::Typescript => extract_with(Tsx, lang, source, ts_compiled()?),
        Lang::Javascript => extract_with(JavaScript, lang, source, js_compiled()?),
        Lang::Python => extract_with(Python, lang, source, python_compiled()?),
        Lang::Go => extract_with(Go, lang, source, go_compiled()?),
    }
}

/// Compile-once accessor for the Rust symbol patterns.
fn rust_compiled() -> Result<&'static [(&'static str, Pattern)]> {
    static CELL: OnceLock<std::result::Result<CompiledPatterns, String>> = OnceLock::new();
    pattern_cache::cached(&CELL, Rust, rust_patterns())
}

/// Compile-once accessor for the TypeScript symbol patterns (Tsx grammar).
fn ts_compiled() -> Result<&'static [(&'static str, Pattern)]> {
    static CELL: OnceLock<std::result::Result<CompiledPatterns, String>> = OnceLock::new();
    pattern_cache::cached(&CELL, Tsx, &ts_patterns())
}

/// Compile-once accessor for the JavaScript symbol patterns.
fn js_compiled() -> Result<&'static [(&'static str, Pattern)]> {
    static CELL: OnceLock<std::result::Result<CompiledPatterns, String>> = OnceLock::new();
    pattern_cache::cached(&CELL, JavaScript, js_patterns())
}

/// Compile-once accessor for the Python symbol patterns.
fn python_compiled() -> Result<&'static [(&'static str, Pattern)]> {
    static CELL: OnceLock<std::result::Result<CompiledPatterns, String>> = OnceLock::new();
    pattern_cache::cached(&CELL, Python, python_patterns())
}

/// Compile-once accessor for the Go symbol patterns.
fn go_compiled() -> Result<&'static [(&'static str, Pattern)]> {
    static CELL: OnceLock<std::result::Result<CompiledPatterns, String>> = OnceLock::new();
    pattern_cache::cached(&CELL, Go, go_patterns())
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

/// Pattern rows shared by the TypeScript and JavaScript tables.
///
/// TypeScript functions may carry a return annotation (`: number`)
/// between the arg list and the body; JavaScript skips it. Both shapes
/// are listed so we recover the function name in either case (the
/// `: $RET` row simply never matches under the JS grammar).
///
/// `export` / `export default` / `async` prefixes need no extra
/// patterns: the wrapped declaration is a child node of the export
/// statement and `find_all` descends into it (probed empirically).
const TS_JS_COMMON: &[(&str, &str)] = &[
    ("function", "function $NAME($$$ARGS): $RET { $$$BODY }"),
    ("function", "function $NAME($$$ARGS) { $$$BODY }"),
    ("class", "class $NAME { $$$BODY }"),
];

/// TypeScript symbol patterns: [`TS_JS_COMMON`] plus the `abstract class`
/// row, which is a distinct node kind under the Tsx grammar. The owned `Vec`
/// is built only at first-call compile time — [`ts_compiled`] caches the
/// resulting [`Pattern`]s, so it is never rebuilt per file.
fn ts_patterns() -> Vec<(&'static str, &'static str)> {
    let mut out = TS_JS_COMMON.to_vec();
    out.push(("class", "abstract class $NAME { $$$BODY }"));
    out
}

fn js_patterns() -> &'static [(&'static str, &'static str)] {
    // JavaScript is exactly the shared table: `abstract class` is
    // rejected by the JS grammar at pattern-compile time.
    TS_JS_COMMON
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

/// Parse `source` under `language` once, then run every pre-compiled
/// `(tag, pattern)` pair against it, invoking `on_match` with the tag and
/// every matched node. Shared by symbol extraction here and import
/// extraction in `crate::graph::imports`.
///
/// The patterns are compiled once per language (see [`pattern_cache`]) and
/// passed in already compiled, so this performs exactly one tree-sitter parse
/// per call and zero pattern compilation — the per-file hot path is parse +
/// match only.
pub(crate) fn for_each_match<L, F>(
    language: L,
    source: &str,
    patterns: &[(&'static str, Pattern)],
    mut on_match: F,
) -> Result<()>
where
    L: LanguageExt + Clone,
    F: FnMut(&str, &NodeMatch<StrDoc<L>>),
{
    let grep = AstGrep::str(source, language);
    let root = grep.root();
    for (tag, pattern) in patterns {
        for matched in root.find_all(pattern) {
            on_match(tag, &matched);
        }
    }
    Ok(())
}

fn extract_with<L: LanguageExt + Clone>(
    language: L,
    lang: Lang,
    source: &str,
    patterns: &[(&'static str, Pattern)],
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
            line_end,
            chunks,
        });
    })?;
    // Collapse symbols that share an identifier *and* a start line: they are
    // indistinguishable under the `code_symbols` UNIQUE(repo, path, symbol,
    // line_start) key, so a later duplicate insert would abort the whole
    // index-code transaction. The usual source is minified/bundled JS, which
    // packs many one-letter `function` expressions onto a single physical
    // line. Keep the first occurrence and drop the rest.
    let mut seen = HashSet::new();
    out.retain(|s| seen.insert((s.name.clone(), s.line)));
    Ok(out)
}
