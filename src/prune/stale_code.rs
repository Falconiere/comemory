//! Stale-code detection: identify referenced source paths that no longer
//! exist on disk.
//!
//! The full v2 design (Task 18 in the plan) is to walk all memories,
//! collect their `references.files` entries, and report the ones that no
//! longer resolve under `repo_root`. For v1 this returns an empty vector:
//! the bookkeeping of "what files each memory references" is not yet wired
//! through the prune pipeline, so on a clean (or any) repo we have nothing
//! to flag. Returning `Ok(vec![])` keeps the CLI surface usable today and
//! makes the upgrade path additive — the signature does not change when the
//! real walker lands.

use std::path::Path;

use crate::prelude::*;

/// Detect referenced source files that have been deleted from `repo_root`.
///
/// Returns an empty vector in the current v1 implementation regardless of
/// whether `repo_root` exists; the eventual implementation will walk
/// memory `references.files` and check each path's existence under
/// `repo_root`.
pub fn detect(_repo_root: &Path) -> Result<Vec<String>> {
    Ok(Vec::new())
}
