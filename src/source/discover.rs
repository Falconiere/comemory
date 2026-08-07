//! Discovery walk over one registered source root: enumerates candidate
//! files, applying the boundary/ignore/exclusion rules from the design
//! spec's "Classification and discovery" section (rules 1-3) before
//! [`classify::classify`] applies rule 4. Read-only — nothing under a
//! source root is ever written, and a directory symlink is never
//! followed.

use std::fs::{self, File};
use std::io::Read as _;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::source::SourceKind;
use crate::source::classify::{self, Classification, SNIFF_WINDOW};

/// One file discovered under a registered source root, already
/// classified. `absolute_path` is the path to open for reading — the
/// symlink's own path for a symlinked file, which transparently follows
/// the link on open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Path relative to the source root (the file's own base name, for
    /// a single-file source).
    pub relative_path: PathBuf,
    /// Absolute path to open for reading.
    pub absolute_path: PathBuf,
    /// How the file classified.
    pub classification: Classification,
}

/// Discover candidate files under `root` (a source root's already
/// canonicalized path), excluding anything under `memories_dir` —
/// Comemory's own managed directory (rule 3). `SourceKind::File` yields
/// exactly `root` (spec: "a single-file source yields exactly that
/// file"), skipping the gitignore/hidden-file rules — an explicit
/// single-file registration overrides them — while still honoring the
/// memories-dir exclusion. `SourceKind::Dir` walks the tree, applying
/// `.gitignore`, hidden-file defaults, and `.comemoryignore` (rule 2)
/// alongside the memories-dir prune. Sorted by `relative_path` for
/// determinism; a walk or read failure on one entry is skipped rather
/// than failing the whole discovery.
pub fn discover(root: &Path, kind: SourceKind, memories_dir: &Path) -> Vec<Candidate> {
    if kind == SourceKind::File {
        return vec![single_file_candidate(root, memories_dir)];
    }
    let mut found = walk_dir(root, memories_dir);
    found.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    found
}

/// Build the one [`Candidate`] a single-file source yields.
fn single_file_candidate(root: &Path, memories_dir: &Path) -> Candidate {
    let relative_path = root
        .file_name()
        .map_or_else(|| root.to_path_buf(), PathBuf::from);
    let classification = if is_within(root, memories_dir) {
        Classification::Ignored
    } else {
        classify_at(root)
    };
    Candidate {
        relative_path,
        absolute_path: root.to_path_buf(),
        classification,
    }
}

/// Walk `root` with `.gitignore`/hidden-file/`.comemoryignore` filtering
/// (rule 2) plus a `memories_dir` prune (rule 3), never following
/// directory symlinks. A plain file is classified directly; a symlinked
/// file is accepted only when [`resolve_symlink_file`] confirms its
/// target stays in-boundary. Unreadable walk entries are skipped.
fn walk_dir(root: &Path, memories_dir: &Path) -> Vec<Candidate> {
    let mut walker = WalkBuilder::new(root);
    walker.standard_filters(true);
    walker.follow_links(false);
    walker.add_custom_ignore_filename(".comemoryignore");
    let pruned = memories_dir.to_path_buf();
    walker.filter_entry(move |entry| !entry.path().starts_with(&pruned));

    let mut out = Vec::new();
    for entry in walker.build().filter_map(|r| match r {
        Ok(e) => Some(e),
        Err(e) => {
            tracing::warn!(error = %e, "discover: walk entry error; skipping");
            None
        }
    }) {
        let path = entry.path();
        if entry.path_is_symlink() {
            if let Some(target) = resolve_symlink_file(path, root) {
                out.push(candidate_of(root, path, &target));
            }
            continue;
        }
        if entry.file_type().is_some_and(|t| t.is_file()) {
            out.push(candidate_of(root, path, path));
        }
    }
    out
}

/// Resolve a symlink at `path` to its canonical target, accepting it
/// only when the target is a regular file that stays inside `root`
/// (spec: "a symlinked file is accepted only when its resolved target
/// stays inside the registered source"). A broken link, a directory
/// target, or an out-of-boundary target is rejected (`None`).
fn resolve_symlink_file(path: &Path, root: &Path) -> Option<PathBuf> {
    let target = fs::canonicalize(path).ok()?;
    if !target.is_file() || !target.starts_with(root) {
        return None;
    }
    Some(target)
}

/// Build a [`Candidate`] for the file at `path` (relative to `root`),
/// sniffing `read_from` for the classifier — the symlink's own path for
/// a symlinked file, which transparently follows the link on open.
fn candidate_of(root: &Path, path: &Path, read_from: &Path) -> Candidate {
    let relative_path = path.strip_prefix(root).unwrap_or(path).to_path_buf();
    Candidate {
        relative_path,
        absolute_path: path.to_path_buf(),
        classification: classify_at(read_from),
    }
}

/// Classify the file at `path`, sniffing up to [`SNIFF_WINDOW`] bytes
/// for [`classify::classify`]. A read failure (permission denied, a
/// race with deletion) yields [`Classification::Unsupported`] rather
/// than failing the walk.
fn classify_at(path: &Path) -> Classification {
    match read_head(path) {
        Ok(head) => classify::classify(path, &head),
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "discover: could not read file head; treating as unsupported",
            );
            Classification::Unsupported
        }
    }
}

/// Read up to [`SNIFF_WINDOW`] bytes from `path`.
fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; SNIFF_WINDOW];
    let n = File::open(path)?.read(&mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// Whether `path` lives inside `memories_dir` (rule 3).
fn is_within(path: &Path, memories_dir: &Path) -> bool {
    path.starts_with(memories_dir)
}

#[cfg(test)]
#[path = "tests/discover.rs"]
mod tests;
