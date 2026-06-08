//! Language registry for the AST layer.
//!
//! Maps file extensions to a small, fixed enum of languages we support
//! (Rust, TypeScript, Tsx, JavaScript, Python). The enum is the only
//! comemory-internal surface code should touch — call sites convert it to
//! the concrete `ast_grep_language` parser inside `extractor` / `pattern`.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    TypeScript,
    /// TypeScript with JSX — separate from `TypeScript` because the tree-sitter
    /// grammar differs (`tsx` parses `<Foo />` as a JSX element, the plain TS
    /// grammar parses it as a `<` comparison and fails on the embedded tag).
    Tsx,
    JavaScript,
    Python,
}

impl Lang {
    /// Map a lower-case file extension (no leading dot) to a language.
    /// Returns `None` for unsupported extensions so callers can skip the file.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "rs" => Some(Self::Rust),
            "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "py" => Some(Self::Python),
            _ => None,
        }
    }

    /// Lowercase, hyphenless name used in stored chunks and search facets.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
            Self::Python => "python",
        }
    }
}

/// Detect the language for a filesystem path by inspecting its extension.
/// Returns `None` when the path has no extension or the extension is not in
/// the supported set, so callers can skip files in a `WalkBuilder` loop.
pub fn detect(path: &std::path::Path) -> Option<Lang> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Lang::from_extension(&ext)
}
