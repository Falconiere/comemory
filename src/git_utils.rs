//! Thin wrapper around `git2` for the auto-reindex pipeline.
//!
//! Three primitives, each scoped to a repo root the caller passes in:
//!
//! * [`current_head`] — resolve the current commit OID as a 40-char hex string.
//!   Used to detect when a repo's HEAD has moved since the last indexing run.
//! * [`changed_files`] — diff two commits and return the set of new-side paths.
//!   Powers incremental `index-code` runs by limiting work to touched files.
//! * [`install_hook`] — write a `.git/hooks/<hook>` script and `chmod +x` it on
//!   unix. Used by `qwick-memory install-hooks` to wire `post-commit`/`post-merge`/
//!   `post-checkout` to a background `index-code --incremental` invocation.
//!
//! All `git2::Error` cases are flattened into [`Error::Other`] via
//! [`map_git_err`] — callers only need to handle our own error enum.

use std::path::Path;

use git2::Repository;

use crate::prelude::*;

/// Lift a `git2::Error` into our `Error::Other` variant so the public API only
/// surfaces a single error type. The git2 message is preserved verbatim.
fn map_git_err(e: git2::Error) -> Error {
    Error::Other(format!("git2: {e}"))
}

/// Return the current HEAD commit OID for the repo containing `repo_root`.
///
/// Uses `Repository::discover`, which walks up the filesystem from the given
/// path until it finds a `.git/` directory — so callers can pass any path
/// inside a working tree, not just the repo root.
///
/// # Errors
/// * No git repo is found by walking up from `repo_root`.
/// * `HEAD` exists but is unborn (no commits yet — `target()` returns `None`).
pub fn current_head(repo_root: &Path) -> Result<String> {
    let repo = Repository::discover(repo_root).map_err(map_git_err)?;
    let head = repo.head().map_err(map_git_err)?;
    let oid = head
        .target()
        .ok_or_else(|| Error::Other("git_utils: HEAD has no target oid (unborn branch?)".into()))?;
    Ok(oid.to_string())
}

/// Return the set of paths whose new-side tree entry changed between two
/// commits. Both `from_sha` and `to_sha` are resolved with `revparse_single`,
/// so callers may pass full OIDs, abbreviated OIDs, refs, or `HEAD~1`-style
/// expressions.
///
/// Paths are reported as the post-rename ("new file") path because
/// downstream incremental indexing cares about which files currently exist in
/// the working tree, not where they used to live.
pub fn changed_files(repo_root: &Path, from_sha: &str, to_sha: &str) -> Result<Vec<String>> {
    let repo = Repository::discover(repo_root).map_err(map_git_err)?;
    let from = repo
        .revparse_single(from_sha)
        .map_err(map_git_err)?
        .peel_to_tree()
        .map_err(map_git_err)?;
    let to = repo
        .revparse_single(to_sha)
        .map_err(map_git_err)?
        .peel_to_tree()
        .map_err(map_git_err)?;
    let diff = repo
        .diff_tree_to_tree(Some(&from), Some(&to), None)
        .map_err(map_git_err)?;
    let mut out = Vec::new();
    diff.foreach(
        &mut |d, _| {
            if let Some(path) = d.new_file().path().and_then(|p| p.to_str()) {
                out.push(path.to_string());
            }
            true
        },
        None,
        None,
        None,
    )
    .map_err(map_git_err)?;
    Ok(out)
}

/// Install (or overwrite) a single git hook under `<repo_root>/.git/hooks/`.
///
/// * `hook` is the bare hook name (`"post-commit"`, `"post-merge"`, …).
/// * `body` is written verbatim — callers are responsible for the shebang.
/// * On unix, the resulting file is `chmod 0755` so git will execute it. The
///   permission bump is feature-gated; on non-unix targets the write still
///   succeeds and git's own platform conventions take over.
///
/// The hooks directory is created if missing (this covers bare worktrees
/// where git hasn't materialized `.git/hooks` yet).
pub fn install_hook(repo_root: &Path, hook: &str, body: &str) -> Result<()> {
    let hooks_dir = repo_root.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;
    let path = hooks_dir.join(hook);
    std::fs::write(&path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&path)?.permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&path, perm)?;
    }
    Ok(())
}
