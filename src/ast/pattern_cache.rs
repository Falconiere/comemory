//! Process-global, compile-once cache of ast-grep [`Pattern`]s.
//!
//! `comemory index-code` runs symbol extraction (and import extraction) once
//! per source file across a whole repo, but the per-language pattern tables
//! are *static*. A compiled [`Pattern`] owns all its data (no lifetime tie to
//! the language instance) and is `Send + Sync`, so each table is compiled once
//! into a call-site `static` [`OnceLock`] and reused across every file and
//! every run. Compilation is fallible, so the cell stores a `Result`: a
//! malformed pattern is surfaced on every lookup, never `unwrap`ped.

use std::sync::OnceLock;

use ast_grep_core::Pattern;
use ast_grep_core::tree_sitter::LanguageExt;

use crate::prelude::*;

/// One pattern table: `(tag, compiled-pattern)` rows in source-table order,
/// so match traversal order is unchanged from the pre-cache code.
pub(crate) type CompiledPatterns = Vec<(&'static str, Pattern)>;

/// Compile-once accessor: initialise `cell` (a call-site `static`
/// [`OnceLock`]) by compiling `raw` under `language` on first call, then
/// return the cached compiled slice. Later calls do no parse work.
///
/// One cell holds exactly one table; a call site that compiles the same rows
/// under different grammars (the shared TypeScript/JavaScript imports) gives
/// each grammar its own cell. `raw` need not be `'static` — it is read only
/// during first-call compilation; only the `&'static str` row tags are
/// retained in the cached output.
pub(crate) fn cached<L: LanguageExt + Clone>(
    cell: &'static OnceLock<std::result::Result<CompiledPatterns, String>>,
    language: L,
    raw: &[(&'static str, &'static str)],
) -> Result<&'static [(&'static str, Pattern)]> {
    match cell.get_or_init(|| compile_patterns(language, raw)) {
        Ok(patterns) => Ok(patterns.as_slice()),
        Err(e) => Err(Error::Other(e.clone())),
    }
}

/// Compile every `(tag, source)` row of `raw` under `language`, preserving
/// order. A single malformed row aborts with an `Err` naming the pattern.
fn compile_patterns<L: LanguageExt + Clone>(
    language: L,
    raw: &[(&'static str, &'static str)],
) -> std::result::Result<CompiledPatterns, String> {
    raw.iter()
        .map(|(tag, src)| {
            Pattern::try_new(src, language.clone())
                .map(|pattern| (*tag, pattern))
                .map_err(|e| format!("ast-grep pattern '{src}' failed: {e:?}"))
        })
        .collect()
}
