//! Canonicalize-and-contain path checks.
//!
//! The one chokepoint that guarantees a caller-supplied path cannot escape the
//! root it was resolved against — through `..`, an absolute path, a NUL, or a
//! symlink. Used by every filesystem-touching surface (`serve`'s file routes,
//! `repo_root`'s `file:<repo>:<path>` ids, and the mutating routes that take a
//! path directly), so the guarantee has a single implementation (#166).

use std::path::{Component, Path, PathBuf};

use crate::prelude::*;

/// Resolve `rel` (a repo-relative path from a `file:<repo>:<path>` id) against
/// the canonical repo `root`, guaranteeing the result stays inside `root`.
///
/// `root` MUST already be canonical (the caller canonicalizes it). The check
/// rejects absolute paths, `..` components, and NUL up front, then
/// canonicalizes the target — resolving symlinks — and asserts it is still
/// prefixed by `root`. For a not-yet-existing file (a fresh `PUT`), the parent
/// directory is canonicalized instead and the filename re-appended, so a
/// symlinked parent escaping the root is still caught.
pub fn resolve_within(root: &Path, rel: &str) -> Result<PathBuf> {
    if rel.is_empty() {
        return Err(Error::BadRequest("empty path".into()));
    }
    if rel.contains('\0') {
        return Err(Error::Forbidden("NUL in path".into()));
    }
    let rel_path = Path::new(rel);
    for comp in rel_path.components() {
        match comp {
            Component::ParentDir => {
                return Err(Error::Forbidden("'..' not allowed in path".into()));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(Error::Forbidden("absolute path not allowed".into()));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    let candidate = root.join(rel_path);
    let canonical = if let Ok(c) = candidate.canonicalize() {
        c
    } else {
        // Not-yet-existing file: canonicalize the parent, re-append name.
        let parent = candidate
            .parent()
            .ok_or_else(|| Error::BadRequest("path has no parent".into()))?;
        let name = candidate
            .file_name()
            .ok_or_else(|| Error::BadRequest("path has no file name".into()))?;
        let parent_canon = parent.canonicalize().map_err(Error::Io)?;
        parent_canon.join(name)
    };
    if !canonical.starts_with(root) {
        return Err(Error::Forbidden("path escapes repo root".into()));
    }
    Ok(canonical)
}

/// Canonicalize `p` and require it inside one of `roots`. Nonexistent path
/// -> `BadRequest` (400); outside every root -> `Forbidden` (403); an empty
/// `roots` slice always forbids (nothing is allowed).
///
/// Unlike [`resolve_within`], `p` may be absolute or cwd-relative — this is
/// the containment check for the mutating routes that take a filesystem path
/// directly (`index-code --path`, `ast --file`, `install-hooks --repo`, a
/// `--golden` file), not a repo-relative `file:<repo>:<path>` id. Every entry
/// in `roots` MUST already be canonical (the caller's job, e.g.
/// `AppState::allowed_roots`); this function does not canonicalize them.
pub fn contain_abs(roots: &[PathBuf], p: &Path) -> Result<PathBuf> {
    let canonical = p
        .canonicalize()
        .map_err(|e| Error::BadRequest(format!("path `{}` is unusable: {e}", p.display())))?;
    if roots.iter().any(|root| canonical.starts_with(root)) {
        Ok(canonical)
    } else {
        Err(Error::Forbidden("path escapes every allowed root".into()))
    }
}

#[cfg(test)]
#[path = "tests/path_containment.rs"]
mod tests;
