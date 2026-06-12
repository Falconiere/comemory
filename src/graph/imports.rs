//! Per-language import extraction and conservative module-to-path resolution.
//!
//! [`extract_imports`] returns the raw module strings a source file imports
//! (deduped, first-seen order within pattern-first/source-second traversal).
//! [`PathIndex`] — built once per materialize run from the repo's indexed
//! file paths — maps one module string onto those paths via
//! [`PathIndex::resolve`] and answers only when the match is unambiguous:
//! zero or two-plus candidates yield `None`, so external packages and
//! ambiguous suffixes drop out naturally instead of being guessed at
//! (spec §2.2). The module is pure — no SQLite, no filesystem — the
//! indexer wires it to real data.
//!
//! Extraction strategy per language (ast-grep patterns were tried first and
//! the outcome of that experiment is recorded honestly):
//!
//! - **Rust** — ast-grep. `use $PATH;` / `pub use $PATH;` / `mod $NAME;` /
//!   `pub mod $NAME;` cover plain and visibility-prefixed forms (verified:
//!   the `pub` patterns also match `pub(crate)` items). Use-tree and rename
//!   text is post-processed: cut at `::{`, `;`, or ` as `, then leading
//!   `crate::` / `self::` / `super::` anchors are stripped.
//! - **TypeScript / JavaScript** — ast-grep. String metavariables match the
//!   `string_fragment` node, which is quote-style sensitive, so every
//!   pattern is listed with both `'…'` and `"…"` variants. `require($ARG)`
//!   binds the whole argument node (quotes included); non-string arguments
//!   such as `require(pluginName)` are filtered out, never guessed at.
//! - **Python** — line parsing. Fallback because the pattern `import $MOD`
//!   drops every name after the first in `import a, b` (verified), while
//!   `import` / `from … import` statements are strictly line-structured.
//! - **Go** — line parsing. Fallback because while `import ($$$SPECS)`
//!   does match a parenthesized block, the `$$$` multi-metavar is not
//!   retrievable through the single-metavar `get_match` API the shared
//!   `for_each_match` helper exposes (verified); a small line state
//!   machine handles both single imports and blocks instead.

use std::collections::{HashMap, HashSet};

use ast_grep_core::tree_sitter::LanguageExt;
use ast_grep_language::{JavaScript, Rust, Tsx};

use crate::ast::extractor::for_each_match;
use crate::ast::languages::Lang;
use crate::prelude::*;

/// Quote characters stripped from / checked against string-literal nodes.
const QUOTES: [char; 3] = ['\'', '"', '`'];

/// Extract the module strings imported by `source`, deduped and order-stable
/// (pattern-first, source-second; first occurrence wins). External packages
/// are included — [`resolve`] is what filters them out, by failing to find a
/// local file. Errors only surface from ast-grep pattern compilation, which
/// would indicate a programming error in the pattern tables.
pub fn extract_imports(lang: Lang, source: &str) -> Result<Vec<String>> {
    let raw = match lang {
        Lang::Rust => rust_imports(source)?,
        // Tsx parses both plain TS and JSX-bearing source (same dispatch
        // choice as `ast::extractor`).
        Lang::Typescript => ts_js_imports(Tsx, source)?,
        Lang::Javascript => ts_js_imports(JavaScript, source)?,
        Lang::Python => python_imports(source),
        Lang::Go => go_imports(source),
    };
    let mut seen = HashSet::new();
    Ok(raw
        .into_iter()
        .filter(|m| !m.is_empty() && seen.insert(m.clone()))
        .collect())
}

/// Resolution state of one lookup key: the single path that owns it, or a
/// tombstone once a second path claims the same key.
enum Candidate {
    /// Exactly one indexed path matches this key.
    Unique(String),
    /// Two or more indexed paths match — conservative: skip, never guess.
    Ambiguous,
}

/// Module-to-path resolution index over a repo's indexed file paths,
/// built ONCE per materialize run (the per-call path rescan it replaces
/// was O(files × imports × paths)).
///
/// Each indexed path contributes the candidate keys of [`candidate_keys`];
/// every segment-aligned suffix of every key lands in the suffix table and
/// the full keys land in the exact table, each entry collapsing to
/// [`Candidate::Ambiguous`] as soon as a second path claims it. A path
/// whose multiple keys share a suffix still counts as one candidate.
pub struct PathIndex {
    /// Segment-aligned key suffix → owning path (suffix-match lookups).
    by_suffix: HashMap<String, Candidate>,
    /// Full candidate key → owning path (exact-match lookups, used by
    /// relative `./` / `../` resolution).
    by_key: HashMap<String, Candidate>,
}

impl PathIndex {
    /// Build the index from the repo's indexed file paths.
    pub fn new(indexed_paths: &[String]) -> PathIndex {
        let mut by_suffix = HashMap::new();
        let mut by_key = HashMap::new();
        for path in indexed_paths {
            for key in candidate_keys(path) {
                insert_candidate(&mut by_key, key.clone(), path);
                let mut start = 0;
                loop {
                    insert_candidate(&mut by_suffix, key[start..].to_string(), path);
                    match key[start..].find('/') {
                        Some(i) => start += i + 1,
                        None => break,
                    }
                }
            }
        }
        PathIndex { by_suffix, by_key }
    }

    /// Resolve a raw module string against the indexed paths.
    ///
    /// Returns `Some(path)` only when exactly one indexed path matches;
    /// zero or multiple candidates return `None` (conservative: skip,
    /// never guess).
    ///
    /// Matching rules, pinned by `tests/graph/imports.rs`:
    ///
    /// 1. The module is normalized to a path fragment: `::` becomes `/`;
    ///    dots become `/` when the module has no slash of its own (Python
    ///    style); leading `./` / `../` segments and trailing slashes are
    ///    stripped.
    /// 2. Each indexed path contributes its path-minus-extension as a key,
    ///    plus its parent directory when the file is a directory entry
    ///    point (`mod.*`, `index.*`, `__init__.*`, or
    ///    `<dir>/<dirname>.<ext>` — the Go package convention).
    /// 3. A path is a candidate when one of its keys ends with the
    ///    fragment at a whole-segment boundary (`store/fts` never matches
    ///    `bookstore/fts`).
    /// 4. Go module-prefix tolerance: when the full fragment has three or
    ///    more slash-separated segments and finds nothing, the leading
    ///    segment (the module name, e.g. `myrepo/`) is dropped once and
    ///    matching retried — only on a clean miss (never to break
    ///    ambiguity), so bare externals cannot collapse onto local files.
    /// 5. When `importing_file` is `Some` and the module starts with `./`
    ///    or `../`, the module is joined against the importer's parent
    ///    directory (normalizing `..`), and an exact key match is required
    ///    instead of a suffix match. `..` escaping the repo root resolves
    ///    to `None`.
    pub fn resolve(&self, module: &str, importing_file: Option<&str>) -> Option<String> {
        if (module.starts_with("./") || module.starts_with("../"))
            && let Some(importer) = importing_file
        {
            return self.resolve_relative(module, importer);
        }
        let fragment = normalize(module)?;
        match self.by_suffix.get(&fragment) {
            Some(Candidate::Unique(p)) => Some(p.clone()),
            Some(Candidate::Ambiguous) => None,
            // Go module-prefix tolerance (rule 4): retry once without the
            // leading segment, only when at least two segments remain.
            None if module.contains('/') && !module.starts_with('.') => {
                let (_, rest) = fragment.split_once('/')?;
                if !rest.contains('/') {
                    return None;
                }
                match self.by_suffix.get(rest) {
                    Some(Candidate::Unique(p)) => Some(p.clone()),
                    _ => None,
                }
            }
            None => None,
        }
    }

    /// Anchored resolution for `./` / `../` modules (rule 5): join against
    /// the importing file's parent directory, normalize `..` segments, and
    /// require an exact candidate-key match. `..` walking above the repo
    /// root yields `None`.
    fn resolve_relative(&self, module: &str, importer: &str) -> Option<String> {
        let dir = importer.rsplit_once('/').map_or("", |(dir, _)| dir);
        let mut segments: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
        for segment in module.split('/') {
            match segment {
                "" | "." => {}
                ".." => {
                    segments.pop()?;
                }
                other => segments.push(other),
            }
        }
        if segments.is_empty() {
            return None;
        }
        match self.by_key.get(&segments.join("/")) {
            Some(Candidate::Unique(p)) => Some(p.clone()),
            _ => None,
        }
    }
}

/// Record `path` as claiming `key`, collapsing to [`Candidate::Ambiguous`]
/// when a DIFFERENT path already owns the key (the same path claiming a
/// key twice — e.g. the Go `x/x.go` convention whose stem and directory
/// keys share the suffix `x` — stays unique).
fn insert_candidate(map: &mut HashMap<String, Candidate>, key: String, path: &str) {
    match map.entry(key) {
        std::collections::hash_map::Entry::Vacant(v) => {
            v.insert(Candidate::Unique(path.to_string()));
        }
        std::collections::hash_map::Entry::Occupied(mut o) => {
            if !matches!(o.get(), Candidate::Unique(p) if p == path) {
                o.insert(Candidate::Ambiguous);
            }
        }
    }
}

/// Rust extraction: `use` / `pub use` / `mod` / `pub mod` patterns in ONE
/// `for_each_match` pass (one tree-sitter parse per file), with the
/// use-path post-processing described in the module doc. The metavar name
/// tags which pattern row hit: `PATH` rows get the use-path trimming,
/// `NAME` rows are taken verbatim.
fn rust_imports(source: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for_each_match(
        Rust,
        source,
        &[
            ("PATH", "use $PATH;"),
            ("PATH", "pub use $PATH;"),
            ("NAME", "mod $NAME;"),
            ("NAME", "pub mod $NAME;"),
        ],
        |var, matched| {
            let Some(node) = matched.get_env().get_match(var) else {
                return;
            };
            let text = node.text().to_string();
            out.push(if var == "PATH" {
                rust_use_path(&text)
            } else {
                text
            });
        },
    )?;
    Ok(out)
}

/// Reduce a matched `use` argument to its module path: cut at the first of
/// `::{` (use tree), `;`, or ` as ` (rename), then strip leading `crate::` /
/// `self::` / `super::` anchors. Bare anchors yield an empty string, which
/// the caller filters out.
fn rust_use_path(text: &str) -> String {
    let mut end = text.len();
    for marker in ["::{", ";", " as "] {
        if let Some(idx) = text.find(marker) {
            end = end.min(idx);
        }
    }
    let mut path = text[..end].trim();
    for prefix in ["crate::", "self::"] {
        if let Some(stripped) = path.strip_prefix(prefix) {
            path = stripped;
        }
    }
    while let Some(stripped) = path.strip_prefix("super::") {
        path = stripped;
    }
    if matches!(path, "crate" | "self" | "super") {
        String::new()
    } else {
        path.to_string()
    }
}

/// TypeScript / JavaScript extraction: ESM `import … from`, bare `import`
/// (each string pattern in both quote styles), and CommonJS `require()` —
/// all in ONE `for_each_match` pass (one tree-sitter parse per file). The
/// metavar name tags which pattern row hit: `SRC` binds the bare
/// `string_fragment`, while `ARG` binds the whole `require` call argument,
/// quotes included — only string literals are kept; dynamic
/// `require(expr)` calls are dropped, never guessed at.
fn ts_js_imports<L: LanguageExt + Clone>(language: L, source: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for_each_match(
        language,
        source,
        &[
            ("SRC", "import $$$SPEC from '$SRC'"),
            ("SRC", "import $$$SPEC from \"$SRC\""),
            ("SRC", "import '$SRC'"),
            ("SRC", "import \"$SRC\""),
            ("ARG", "require($ARG)"),
        ],
        |var, matched| {
            let Some(node) = matched.get_env().get_match(var) else {
                return;
            };
            let text = node.text().to_string();
            if var == "ARG" {
                let stripped = text.trim_matches(QUOTES);
                if stripped.len() < text.len() && !stripped.contains(QUOTES) {
                    out.push(stripped.to_string());
                }
            } else {
                out.push(text);
            }
        },
    )?;
    Ok(out)
}

/// Python extraction by line parsing: `import a[, b][ as c]` and
/// `from X import …` (the module is always on the statement's first line,
/// even for parenthesized import lists).
fn python_imports(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("from ") {
            if let Some((module, _)) = rest.split_once(" import") {
                out.push(module.trim().to_string());
            }
        } else if let Some(rest) = trimmed.strip_prefix("import ") {
            for part in rest.split(',') {
                if let Some(name) = part.split_whitespace().next() {
                    out.push(name.trim_end_matches(';').to_string());
                }
            }
        }
    }
    out
}

/// Go extraction by line parsing: single `import "x"` (optionally aliased)
/// plus parenthesized import blocks via a one-flag state machine.
fn go_imports(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_block = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if in_block {
            if trimmed.starts_with(')') {
                in_block = false;
            } else if !trimmed.starts_with("//")
                && let Some(path) = first_quoted(trimmed)
            {
                out.push(path);
            }
        } else if let Some(rest) = trimmed.strip_prefix("import")
            && (rest.starts_with('(') || rest.starts_with(char::is_whitespace))
        {
            let rest = rest.trim_start();
            if rest.starts_with('(') && !rest.contains(')') {
                in_block = true;
            } else if let Some(path) = first_quoted(rest) {
                out.push(path);
            }
        }
    }
    out
}

/// First `"…"`-quoted substring of `line`, if any.
fn first_quoted(line: &str) -> Option<String> {
    let start = line.find('"')?;
    let rest = &line[start + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Normalize a module string to a slash-separated path fragment, or `None`
/// when nothing remains (e.g. a bare `.`).
fn normalize(module: &str) -> Option<String> {
    let mut module = module.trim().trim_end_matches('/');
    loop {
        if let Some(rest) = module.strip_prefix("./") {
            module = rest;
        } else if let Some(rest) = module.strip_prefix("../") {
            module = rest;
        } else {
            break;
        }
    }
    let fragment = if module.contains("::") {
        module.replace("::", "/")
    } else if !module.contains('/') {
        // Dotted module path (Python). Slash-bearing modules keep their
        // dots — they may be extensions (`./db.js`) or version segments.
        module.replace('.', "/")
    } else {
        module.to_string()
    };
    let fragment = fragment.trim_matches('/');
    if fragment.is_empty() {
        None
    } else {
        Some(fragment.to_string())
    }
}

/// Match keys for one indexed path: the path minus its extension, plus the
/// parent directory when the file is a directory entry point (`mod.*`,
/// `index.*`, `__init__.*`, or the Go `<dir>/<dirname>.<ext>` convention).
fn candidate_keys(path: &str) -> Vec<String> {
    let stem = strip_extension(path);
    let mut keys = vec![stem.to_string()];
    if let Some((dir, file)) = stem.rsplit_once('/') {
        let dir_name = dir.rsplit('/').next().unwrap_or(dir);
        if matches!(file, "mod" | "index" | "__init__") || file == dir_name {
            keys.push(dir.to_string());
        }
    }
    keys
}

/// Strip a trailing `.ext` from the final path segment, leaving dot-files
/// (`.gitignore`) and extension-less paths untouched.
fn strip_extension(path: &str) -> &str {
    let file_start = path.rfind('/').map_or(0, |idx| idx + 1);
    match path[file_start..].rfind('.') {
        Some(rel) if rel > 0 => &path[..file_start + rel],
        _ => path,
    }
}
