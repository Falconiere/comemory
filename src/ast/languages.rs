//! Registry of compiled-in ast-grep languages.
//!
//! comemory ships with exactly five tree-sitter parsers: rust, typescript
//! (which also covers `.tsx`), javascript, python, and go. The enum is the
//! only comemory-internal surface code should touch — call sites convert it
//! to the concrete `ast_grep_language` parser inside `extractor` / `pattern`.

use std::path::Path;

/// Compiled-in language for ast-grep dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    /// Rust (`*.rs`).
    Rust,
    /// TypeScript (`*.ts`, `*.tsx`). `.tsx` files reuse this variant — the
    /// extractor / pattern dispatch picks the Tsx grammar internally so JSX
    /// still parses correctly.
    Typescript,
    /// JavaScript (`*.js`, `*.jsx`, `*.mjs`, `*.cjs`).
    Javascript,
    /// Python (`*.py`).
    Python,
    /// Go (`*.go`).
    Go,
}

impl Lang {
    /// Canonical, hyphenless name used in stored chunks and search facets.
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Typescript => "typescript",
            Lang::Javascript => "javascript",
            Lang::Python => "python",
            Lang::Go => "go",
        }
    }

    /// Parse a CLI `--lang` value. Accepts the canonical name plus common
    /// short aliases (`rs`, `ts`, `tsx`, `js`, `py`). Returns `None` for
    /// anything outside the compiled-in set so the CLI can render a
    /// helpful error.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rust" | "rs" => Some(Lang::Rust),
            "typescript" | "ts" | "tsx" => Some(Lang::Typescript),
            "javascript" | "js" | "jsx" => Some(Lang::Javascript),
            "python" | "py" => Some(Lang::Python),
            "go" => Some(Lang::Go),
            _ => None,
        }
    }
}

/// Canonical names of every language comemory's ast-grep stack supports.
/// Surfaced in CLI error messages so callers see the accepted set.
pub fn supported() -> &'static [&'static str] {
    &["rust", "typescript", "javascript", "python", "go"]
}

/// Detect the language for a filesystem path by inspecting its extension.
/// Returns `None` when the path has no extension or the extension is not in
/// the supported set, so callers can skip files in a `WalkBuilder` loop.
pub fn detect(path: &Path) -> Option<Lang> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Some(Lang::Rust),
        "ts" | "tsx" => Some(Lang::Typescript),
        "js" | "jsx" | "mjs" | "cjs" => Some(Lang::Javascript),
        "py" => Some(Lang::Python),
        "go" => Some(Lang::Go),
        _ => None,
    }
}
