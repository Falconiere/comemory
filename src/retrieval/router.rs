//! Query classifier. Pure function over the raw query string — no I/O, no
//! state — so router behaviour stays reproducible and unit-testable.

/// Which retrieval branch the pipeline should take for a given query.
///
/// - [`Route::Symbol`]: single identifier-like token (`handleLogin`,
///   `run_migration`) — code-symbol lookups in later tasks.
/// - [`Route::FtsFirst`]: empty or very short query — favour lexical/FTS.
/// - [`Route::Hybrid`]: natural-language query — run vector + FTS together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Symbol,
    FtsFirst,
    Hybrid,
}

/// Classify `query` into a [`Route`]. Whitespace-only queries collapse to
/// `FtsFirst`; single-token identifier-looking inputs route to `Symbol`;
/// 1-2 word inputs route to `FtsFirst`; everything else is `Hybrid`.
pub fn classify(query: &str) -> Route {
    let q = query.trim();
    if q.is_empty() {
        return Route::FtsFirst;
    }
    let single = !q.contains(char::is_whitespace);
    let identifier = q
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':'));
    if single && identifier {
        return Route::Symbol;
    }
    if q.split_whitespace().count() <= 2 {
        return Route::FtsFirst;
    }
    Route::Hybrid
}
