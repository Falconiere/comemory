//! The two derived navigation fields a memory listing reports: the title and
//! the absolute path of the markdown file. Neither is stored — both are
//! recomputed from the body and from `store::memory_meta`'s `md_path` — so
//! every surface that shows a memory (`comemory search`, `list`, `show`,
//! `GET /trash`, the prune report, the graph node panel) derives them here
//! instead of re-stating the rule.

use std::path::{Path, PathBuf};

use crate::store::memory_meta::MemoryMeta;

/// First non-empty trimmed line of `body` — a human-readable title. Empty
/// when the body has no non-blank line. This is the *definition* of a
/// memory's title: [`super::save::fold_title`] compares a supplied title
/// against it to keep a round-tripped save idempotent, and every listing
/// reports the same string (Binding Rule 1).
pub(crate) fn title_of(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// Resolve a memory's stored `md_path` against `data_dir` into an absolute
/// path string. Returns an empty string when the metadata is absent (a raced
/// soft-delete or rebuild). `Path::join` returns an absolute `md_path`
/// unchanged and joins a relative one, so this is correct whichever form the
/// writer stored.
pub(crate) fn abs_path(entry: Option<&MemoryMeta>, data_dir: &Path) -> String {
    match entry {
        Some(m) => PathBuf::from(data_dir)
            .join(&m.md_path)
            .to_string_lossy()
            .into_owned(),
        None => String::new(),
    }
}
