//! AST layer: ast-grep-based symbol extraction + pattern search.

pub mod chunk;
pub mod extractor;
pub mod languages;
pub mod pattern;
pub(crate) mod pattern_cache;

pub use extractor::{ExtractedSymbol, extract};
pub use languages::{Lang, detect};
