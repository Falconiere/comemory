//! Thin wrapper around `git2` for the auto-reindex pipeline.
//!
//! Three primitives, each scoped to a repo root the caller passes in:
//!
//! * [`current_head`] — resolve the current commit OID as a 40-char hex string.
//!   Used to detect when a repo's HEAD has moved since the last indexing run.
//! * [`changed_files`] — diff two commits and return the set of new-side paths.
//!   Powers incremental `index-code` runs by limiting work to touched files.
//! * [`install_hook`] — write a `<hooks_dir>/<hook>` script and `chmod +x` it
//!   on unix. Used by `comemory install-hooks` to wire `post-commit`/
//!   `post-merge`/`post-checkout` to a background `index-code --incremental`
//!   invocation. [`hooks_dir`] is the shared common-dir `hooks/`, so a hook
//!   installed from a linked worktree lands where git actually runs it.
//! * [`repo_label`] — the ONE rule turning a checkout into a repo label: the
//!   main working tree's basename, shared by every linked `git worktree`.
//!   Every label the binary infers (lazy reindex, the hook script, the code
//!   rerank working set, document sources, `POST /repos`) goes through it.
//!   [`is_linked_worktree`] is its narrowing companion: "is this checkout a
//!   second one of some other repository?", which `index-code` asks before it
//!   would create a repo label that has never been seen before.
//! * [`hook_outdated`] — whether a hook we wrote predates the body this
//!   binary ships, so `install-hooks` can replace its own stale scripts
//!   instead of leaving a pre-worktree-rule hook running forever.
//!
//! All `git2::Error` cases are flattened into [`Error::Other`] via
//! [`map_git_err`] — callers only need to handle our own error enum.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use git2::Repository;

use crate::prelude::*;

/// Lift a `git2::Error` into our `Error::Other` variant so the public API only
/// surfaces a single error type. The git2 message is preserved verbatim.
pub(crate) fn map_git_err(e: git2::Error) -> Error {
    Error::Other(format!("git2: {e}"))
}

/// Diff `old` (`None` = empty tree, for root commits) against `new` and
/// collect the new-side path of every delta. Shared by [`changed_files`]
/// (which resolves rev strings to trees first) and
/// `graph::cochange::commit_changed_paths` (which passes revwalk-held
/// trees directly).
pub(crate) fn collect_diff_paths(
    repo: &Repository,
    old: Option<&git2::Tree>,
    new: &git2::Tree,
) -> Result<Vec<String>> {
    let diff = repo
        .diff_tree_to_tree(old, Some(new), None)
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

/// Resolve the HEAD commit OID of an already-open repository as a 40-char
/// hex string. Shared by [`current_head`] (which discovers the repo from a
/// path first) and `graph::cochange::mine_cochange` (which already holds an
/// open `Repository`), so the two callers cannot drift on the unborn-HEAD
/// handling.
///
/// # Errors
/// * `HEAD` cannot be resolved.
/// * `HEAD` exists but is unborn (no commits yet — `target()` returns `None`).
pub(crate) fn head_oid(repo: &Repository) -> Result<String> {
    let head = repo.head().map_err(map_git_err)?;
    let oid = head
        .target()
        .ok_or_else(|| Error::Other("git_utils: HEAD has no target oid (unborn branch?)".into()))?;
    Ok(oid.to_string())
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
    head_oid(&repo)
}

/// Blob OID (40-char hex) of repo-root-relative `rel_path` in the HEAD tree of
/// the repo containing `repo_root`. `Ok(None)` for every benign "no committed
/// blob" case so save can degrade a reference to `unpinned` without erroring:
/// no repo, unborn HEAD, path absent from the HEAD tree (untracked), or the
/// entry is a directory. Other git2 failures (corrupt store, I/O) propagate.
pub fn blob_oid_at_head(repo_root: &Path, rel_path: &str) -> Result<Option<String>> {
    let Ok(repo) = Repository::discover(repo_root) else {
        return Ok(None);
    };
    // Unborn HEAD (no commits yet) surfaces as an error from `head()`.
    let Ok(head) = repo.head() else {
        return Ok(None);
    };
    let tree = head.peel_to_tree().map_err(map_git_err)?;
    // `GIT_ENOTFOUND`: the path is not in the HEAD tree.
    let Ok(entry) = tree.get_path(Path::new(rel_path)) else {
        return Ok(None);
    };
    if entry.kind() != Some(git2::ObjectType::Blob) {
        return Ok(None);
    }
    Ok(Some(entry.id().to_string()))
}

/// Return the short branch name (e.g. `"main"`) currently checked out in the
/// repo containing `repo_root`, or `None` when HEAD is detached (points at a
/// commit rather than a branch) or unborn.
///
/// # Errors
/// * No git repo is found by walking up from `repo_root`.
pub fn current_branch(repo_root: &Path) -> Result<Option<String>> {
    let repo = Repository::discover(repo_root).map_err(map_git_err)?;
    // Unborn HEAD has no resolvable branch yet.
    let Ok(head) = repo.head() else {
        return Ok(None);
    };
    if !head.is_branch() {
        return Ok(None);
    }
    // git2 0.21: `shorthand()` returns `Result` (errors only on invalid
    // UTF-8); treat that the same as the prior `None` case.
    Ok(head.shorthand().ok().map(std::string::ToString::to_string))
}

/// The label comemory files a checkout under: the basename of the **main**
/// working tree, so every linked `git worktree` of one repository shares
/// one label with it instead of minting a per-worktree scope (which split
/// the code index and the console's repo list by checkout directory).
///
/// Derived from `commondir()` — for a linked worktree that is the main
/// worktree's `.git`, for the main worktree its own — so `<main>/.git`
/// yields `<main>`. A common dir not named `.git` (a bare repository, a
/// custom `GIT_DIR`) falls back to the working-tree basename, which is
/// `None` for a bare repo. Mirrors `comemory_repo_key` in
/// `integrations/agent/lib/repo-scope.sh`, so the binary and the plugin
/// wrapper mint the same key.
pub fn repo_label(repo: &Repository) -> Option<String> {
    let dir = main_worktree_dir(repo)?;
    dir.file_name().and_then(OsStr::to_str).map(str::to_string)
}

/// The **main** working tree's directory for `repo`, from any checkout of it.
///
/// Derived from `commondir()` — for a linked worktree that is the main
/// worktree's `.git`, for the main worktree its own — so `<main>/.git` yields
/// `<main>`. A common dir not named `.git` (a bare repository, a custom
/// `GIT_DIR`) falls back to this checkout's own working tree, which is `None`
/// for a bare repo.
///
/// [`repo_label`] is its basename. It is also the directory a repository is
/// *recorded* at (`repo_marker.root_path`): a linked worktree is a temporary
/// checkout that `git worktree remove` deletes, so pointing a repo's stored
/// root at one would leave `serve::repo_root` resolving file ids against a
/// directory that no longer exists.
pub fn main_worktree_dir(repo: &Repository) -> Option<PathBuf> {
    let common = repo.commondir();
    let main_worktree = (common.file_name() == Some(OsStr::new(".git")))
        .then(|| common.parent())
        .flatten();
    main_worktree
        .or_else(|| repo.workdir())
        .map(Path::to_path_buf)
}

/// [`repo_label`] for the repository containing `start` (walking up like
/// `Repository::discover`), or `None` when `start` is not inside one.
pub fn repo_label_at(start: &Path) -> Option<String> {
    let repo = Repository::discover(start).ok()?;
    repo_label(&repo)
}

/// Whether `repo` is a **linked** worktree (`git worktree add`) rather than
/// the main one. A linked worktree's git dir is
/// `<main>/.git/worktrees/<name>` while its common dir is `<main>/.git`; for
/// the main worktree the two are the same directory.
///
/// A linked worktree is a second checkout of one repository, never a
/// repository of its own — [`crate::api::index_code`] uses this to refuse to
/// mint a new repo label for one.
pub fn is_linked_worktree(repo: &Repository) -> bool {
    let git_dir = repo.path();
    let common = repo.commondir();
    // The main worktree is the common case and git2 hands both paths back with
    // the same spelling there, so settle it without touching the filesystem.
    if git_dir == common {
        return false;
    }
    // Only a genuine difference is worth two `canonicalize` calls: git2 returns
    // whatever spelling the repo was opened with, so `/tmp/...` vs
    // `/private/tmp/...` (macOS) or a trailing separator must not read as one.
    match (git_dir.canonicalize(), common.canonicalize()) {
        (Ok(a), Ok(b)) => a != b,
        // An uncanonicalizable path is not evidence of a linked worktree.
        _ => false,
    }
}

/// [`is_linked_worktree`] for the repository containing `start`, or `false`
/// when `start` is not inside one — the degrade-to-`false` contract
/// [`hook_installed`] uses, since "is this a worktree" is only ever a
/// narrowing check.
pub fn is_linked_worktree_at(start: &Path) -> bool {
    Repository::discover(start).is_ok_and(|repo| is_linked_worktree(&repo))
}

/// The directory git runs `repo_root`'s hooks from: `<commondir>/hooks`,
/// which a linked worktree shares with its main worktree (its own `.git` is
/// a file pointing at the common dir, so `<worktree>/.git/hooks` neither
/// exists nor could be created). Falls back to `<repo_root>/.git/hooks` when
/// `repo_root` itself is not an openable repository — deliberately `open`,
/// not `discover`, so a non-repo directory never has hooks written into
/// some ancestor's `.git`.
pub fn hooks_dir(repo_root: &Path) -> PathBuf {
    Repository::open(repo_root).map_or_else(
        |_| repo_root.join(".git").join("hooks"),
        |repo| repo.commondir().join("hooks"),
    )
}

/// Return the URL configured for `name` remote (e.g. `"origin"`) in the repo
/// containing `repo_root`, or `None` when the repo cannot be discovered,
/// carries no such remote, or the URL is not valid UTF-8. Every failure mode
/// here is benign — resolving a repo's remote is advisory (`api::repos`'s
/// inventory row), never a hard requirement of any caller — so this returns
/// a plain `Option` rather than propagating, mirroring [`current_branch`]'s
/// contract one step further.
pub fn remote_url(repo_root: &Path, name: &str) -> Option<String> {
    let repo = Repository::discover(repo_root).ok()?;
    let remote = repo.find_remote(name).ok()?;
    remote.url().ok().map(std::string::ToString::to_string)
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
    collect_diff_paths(&repo, Some(&from), &to)
}

/// Install (or overwrite) a single git hook under [`hooks_dir`]`(repo_root)`.
///
/// * `hook` is the bare hook name (`"post-commit"`, `"post-merge"`, …).
/// * `body` is written verbatim — callers are responsible for the shebang.
/// * On unix, the resulting file is `chmod 0755` so git will execute it. The
///   permission bump is feature-gated; on non-unix targets the write still
///   succeeds and git's own platform conventions take over.
///
/// The hooks directory is created if missing (this covers fresh repos
/// where git hasn't materialized `.git/hooks` yet).
pub fn install_hook(repo_root: &Path, hook: &str, body: &str) -> Result<()> {
    let hooks_dir = hooks_dir(repo_root);
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

/// Substring every hook script [`install_hook`] writes contains — the
/// backgrounded `index-code` invocation line shared (by hand — the two
/// literals are kept in sync manually, there is no single source) with
/// `api::install_hooks::SCRIPT`. [`hook_installed`] looks for this
/// substring rather than requiring a byte-identical body, so a hook written
/// by an older or newer revision of the reindex script is still recognized
/// as comemory-managed, while a hand-written third-party hook (which won't
/// contain it) is not.
pub(crate) const HOOK_MARKER: &str = "comemory index-code";

/// The reindex hook body written into `.git/hooks/<hook>`. The single
/// definition shared by `api::install_hooks` (which writes all three hooks
/// at once) and `api::hooks`'s per-hook `--enable`, so the two cannot drift
/// (Binding Rule 1). Must always contain [`HOOK_MARKER`], which is how
/// [`hook_installed`] recognizes a comemory-written hook. The trailing `&`
/// detaches the indexer so git's hook runner returns immediately.
///
/// The label is derived the way [`repo_label`] derives it — the basename of
/// the common dir's parent, so a commit in a linked worktree refreshes the
/// main repo's index rather than minting a `<worktree-name>` repo. The
/// `--show-toplevel` basename is only the fallback for a common dir not
/// named `.git` (bare layouts) or a git too old for `--path-format`.
pub const REINDEX_HOOK_SCRIPT: &str = "#!/usr/bin/env bash\n\
                      ROOT=\"$(git rev-parse --show-toplevel 2>/dev/null)\"\n\
                      [ -z \"$ROOT\" ] && exit 0\n\
                      COMMON=\"$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null)\"\n\
                      case \"$COMMON\" in\n\
                        */.git) REPO=\"$(basename \"$(dirname \"$COMMON\")\")\" ;;\n\
                        *) REPO=\"$(basename \"$ROOT\")\" ;;\n\
                      esac\n\
                      ( comemory index-code --repo \"$REPO\" --path \"$ROOT\" >/dev/null 2>&1 & )\n\
                      exit 0\n";

/// The body of `<hooks_dir>/<hook>` for `repo_root`, or `None` when it is
/// missing, unreadable, or not UTF-8. The single read behind
/// [`hook_installed`] and [`hook_outdated`], so the two cannot disagree
/// about what is on disk (Binding Rule 1).
fn hook_body(repo_root: &Path, hook: &str) -> Option<String> {
    std::fs::read_to_string(hooks_dir(repo_root).join(hook)).ok()
}

/// Whether `<hooks_dir>/<hook>` exists for `repo_root` and carries
/// [`HOOK_MARKER`] — the on-disk state `comemory hooks` reports (no DB
/// table involved). A read-side probe: any I/O failure (missing file,
/// unreadable, non-UTF8 content) reports `false` rather than erroring,
/// matching [`current_branch`]/[`remote_url`]'s degrade-to-`None` contract
/// one step further — listing hook state is inherently best-effort.
pub fn hook_installed(repo_root: &Path, hook: &str) -> bool {
    hook_body(repo_root, hook).is_some_and(|body| body.contains(HOOK_MARKER))
}

/// Whether `<hooks_dir>/<hook>` is a comemory-written hook whose body is no
/// longer the one this binary ships — a hook installed by an older release.
///
/// This exists because [`hook_installed`] deliberately matches on
/// [`HOOK_MARKER`] alone: a hook written before the worktree-label rule
/// (which passed `basename "$(git rev-parse --show-toplevel)"` as `--repo`)
/// still contains the marker, so it counted as installed and no release ever
/// replaced it. Every commit and every `git worktree add` in such a repo kept
/// minting a `<worktree-dir>` repo label. [`crate::api::install_hooks`] uses
/// this to rewrite our own outdated hooks without `--force`.
///
/// A file we did not write (no marker) is never "outdated" — it is foreign,
/// and replacing it is the caller's explicit `--force` decision.
pub fn hook_outdated(repo_root: &Path, hook: &str) -> bool {
    hook_body(repo_root, hook)
        .is_some_and(|body| body.contains(HOOK_MARKER) && body != REINDEX_HOOK_SCRIPT)
}

/// Remove `<hooks_dir>/<hook>` for `repo_root`, if present.
///
/// Idempotent — `Ok(())` whether or not the file existed — so `comemory
/// hooks --disable` is safe to call repeatedly (spec: "idempotent and
/// reversible"). Unlike [`hook_installed`]'s read-side degrade, this is a
/// caller-requested write: any I/O failure other than the file already
/// being gone propagates as [`Error::Io`] rather than being silenced.
pub fn remove_hook(repo_root: &Path, hook: &str) -> Result<()> {
    let path = hooks_dir(repo_root).join(hook);
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(&path)?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/git_utils.rs"]
mod tests;
