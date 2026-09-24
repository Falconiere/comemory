#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration tests for `comemory::domains::code::git_utils`. We shell out to the real `git`
//! binary to construct fixtures (rather than driving `git2` directly) so the
//! test exercises the same on-disk layout a user repo would have — including
//! the `.git/` directory `Repository::discover` looks for.

use std::process::Command;

use comemory::domains::code::git_utils::{
    CheckoutKind, REINDEX_HOOK_SCRIPT, blob_oid_at_head, changed_files, checkout_kind,
    current_branch, current_head, hook_installed, hook_outdated, hooks_dir, install_hook,
    is_linked_worktree, remove_hook, repo_label, repo_label_at,
};
use tempfile::TempDir;

use crate::test_common::git_worktree::add_worktree;

/// Build a git repo in `dir` with a single commit. Returns the path so the
/// caller can keep the `TempDir` alive. Panics on any git failure because the
/// test environment is broken if `git init`/`git commit` can't succeed.
fn make_repo_with_one_commit(dir: &TempDir) {
    make_repo_at(dir.path());
}

/// [`make_repo_with_one_commit`] at an arbitrary (created) path, for the
/// worktree tests that need the main repo to carry a meaningful basename.
fn make_repo_at(p: &std::path::Path) {
    std::fs::create_dir_all(p).expect("create repo dir");
    run_git(p, &["init", "--quiet"]);
    // Configure identity locally so the commit succeeds even on CI hosts where
    // no global git identity is set.
    run_git(p, &["config", "user.email", "test@qwick.local"]);
    run_git(p, &["config", "user.name", "qwick-test"]);
    // Pin the default branch so behaviour is reproducible across git versions
    // that default to either `master` or `main`.
    run_git(p, &["checkout", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "hi").expect("write a.txt");
    run_git(p, &["add", "a.txt"]);
    run_git(p, &["commit", "-q", "-m", "initial"]);
}

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn current_head_returns_oid_after_commit() {
    let tmp = TempDir::new().expect("tempdir");
    make_repo_with_one_commit(&tmp);

    let head = current_head(tmp.path()).expect("current_head");
    assert_eq!(
        head.len(),
        40,
        "git OID should be 40 hex chars, got {head:?}"
    );
    assert!(
        head.chars().all(|c| c.is_ascii_hexdigit()),
        "non-hex chars in {head}"
    );
}

#[test]
fn changed_files_reports_new_path_between_commits() {
    let tmp = TempDir::new().expect("tempdir");
    make_repo_with_one_commit(&tmp);
    let from = current_head(tmp.path()).expect("head after first commit");

    std::fs::write(tmp.path().join("b.txt"), "hello").expect("write b.txt");
    run_git(tmp.path(), &["add", "b.txt"]);
    run_git(tmp.path(), &["commit", "-q", "-m", "add b"]);
    let to = current_head(tmp.path()).expect("head after second commit");

    let files = changed_files(tmp.path(), &from, &to).expect("changed_files");
    assert!(
        files.iter().any(|p| p == "b.txt"),
        "expected b.txt in {files:?}"
    );
}

#[test]
fn install_hook_writes_executable_script() {
    let tmp = TempDir::new().expect("tempdir");
    make_repo_with_one_commit(&tmp);

    install_hook(tmp.path(), "post-commit", "#!/usr/bin/env bash\necho hi\n")
        .expect("install_hook");
    let path = tmp.path().join(".git/hooks/post-commit");
    assert!(
        path.exists(),
        "hook file should exist at {}",
        path.display()
    );
    let body = std::fs::read_to_string(&path).expect("read hook");
    assert!(body.contains("echo hi"), "hook body wrong: {body:?}");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        // Lower 9 bits = rwx triples; we wrote 0o755.
        assert_eq!(mode & 0o777, 0o755, "expected 0755, got {:o}", mode & 0o777);
    }
}

/// `<tmp>/parent-repo` (one commit) plus a linked worktree at
/// `<tmp>/parent-repo-feature-42` on branch `feature-42`.
fn main_and_linked_worktree(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let main = tmp.path().join("parent-repo");
    make_repo_at(&main);
    let wt = tmp.path().join("parent-repo-feature-42");
    add_worktree(&main, &wt, "feature-42");
    assert!(
        wt.join(".git").is_file(),
        "a linked worktree's .git is a file, not a directory"
    );
    (main, wt)
}

#[test]
fn repo_label_is_the_main_worktree_basename_from_every_checkout() {
    let tmp = TempDir::new().expect("tempdir");
    let (main, wt) = main_and_linked_worktree(&tmp);
    let nested = wt.join("src").join("deep");
    std::fs::create_dir_all(&nested).expect("nested dir");

    assert_eq!(repo_label_at(&main).as_deref(), Some("parent-repo"));
    assert_eq!(
        repo_label_at(&wt).as_deref(),
        Some("parent-repo"),
        "a linked worktree must not mint its own directory name as the label"
    );
    assert_eq!(repo_label_at(&nested).as_deref(), Some("parent-repo"));
    assert_eq!(
        repo_label_at(tmp.path()),
        None,
        "outside any repository there is no label"
    );
}

#[test]
fn repo_label_is_none_for_a_bare_repository() {
    let tmp = TempDir::new().expect("tempdir");
    let bare = tmp.path().join("bare.git");
    std::fs::create_dir_all(&bare).expect("bare dir");
    run_git(&bare, &["init", "--quiet", "--bare"]);
    let repo = git2::Repository::open(&bare).expect("open bare");
    assert_eq!(repo_label(&repo), None);
}

#[test]
fn hooks_of_a_linked_worktree_live_in_the_shared_common_dir() {
    let tmp = TempDir::new().expect("tempdir");
    let (main, wt) = main_and_linked_worktree(&tmp);

    install_hook(&wt, "post-commit", REINDEX_HOOK_SCRIPT).expect("install from worktree");

    let shared = main.join(".git").join("hooks");
    assert_eq!(
        hooks_dir(&wt)
            .canonicalize()
            .expect("canonical wt hooks dir"),
        shared.canonicalize().expect("canonical shared hooks dir")
    );
    assert!(shared.join("post-commit").is_file());
    assert!(hook_installed(&wt, "post-commit"));
    assert!(
        hook_installed(&main, "post-commit"),
        "the main worktree sees the hook the linked worktree installed"
    );

    remove_hook(&wt, "post-commit").expect("remove from worktree");
    assert!(!hook_installed(&main, "post-commit"));
    assert!(!shared.join("post-commit").exists());
}

#[test]
fn hooks_dir_falls_back_to_dot_git_hooks_outside_a_repository() {
    let tmp = TempDir::new().expect("tempdir");
    assert_eq!(hooks_dir(tmp.path()), tmp.path().join(".git").join("hooks"));
}

/// The shipped hook is valid bash, still carries the marker `hook_installed`
/// recognizes, and hands its checkout to the sync pass rather than indexing on
/// its own — the pass indexes under the main worktree's label and holds the
/// pass lock while it does. That it really does so, from a real commit in a
/// real linked worktree with the real binary, is `tests/cli__sync_auto.rs`.
#[test]
fn reindex_hook_is_valid_bash_that_hands_its_checkout_to_the_sync_pass() {
    let tmp = TempDir::new().expect("tempdir");
    let script = tmp.path().join("post-commit");
    std::fs::write(&script, REINDEX_HOOK_SCRIPT).expect("write hook");
    let out = Command::new("bash")
        .arg("-n")
        .arg(&script)
        .output()
        .expect("spawn bash -n");
    assert!(
        out.status.success(),
        "hook body must parse: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(REINDEX_HOOK_SCRIPT.contains(super::HOOK_MARKER));
    assert!(REINDEX_HOOK_SCRIPT.contains(r#"sync --action auto --path "$ROOT""#));
    assert!(
        !REINDEX_HOOK_SCRIPT.contains("\"$CM\" index-code"),
        "the hook must not run an index of its own beside the pass"
    );
    for dir in [
        "$HOME/.cargo/bin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "$HOME/.local/bin",
    ] {
        assert!(REINDEX_HOOK_SCRIPT.contains(dir), "fallback {dir} missing");
    }
}

/// With `HOME` unset the fallback candidates degrade to `/.cargo/bin/…` and
/// `/.local/bin/…`; the hook must still exit 0, so the commit that fired it
/// succeeds. `COMEMORY_DATA_DIR` points into the temp dir in case a
/// `comemory` does turn up under `/opt/homebrew/bin` or `/usr/local/bin`.
#[test]
fn reindex_hook_never_fails_a_commit_when_home_is_unset() {
    let tmp = TempDir::new().expect("tempdir");
    make_repo_with_one_commit(&tmp);
    install_hook(tmp.path(), "post-commit", REINDEX_HOOK_SCRIPT).expect("install hook");
    std::fs::write(tmp.path().join("b.txt"), "b").expect("write b.txt");
    run_git(tmp.path(), &["add", "b.txt"]);

    let out = Command::new("git")
        .args(["commit", "-q", "-m", "no home"])
        .current_dir(tmp.path())
        .env_remove("HOME")
        .env("PATH", "/usr/bin:/bin")
        .env("COMEMORY_DATA_DIR", tmp.path().join("data"))
        .output()
        .expect("spawn git commit");

    assert!(
        out.status.success(),
        "a hook must never fail the commit: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Path to the real comemory checkout this test crate lives in — a genuine git
/// repo with committed files. `CARGO_MANIFEST_DIR` is the crate root.
fn comemory_repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn blob_oid_at_head_returns_hex_for_tracked_file() {
    let root = comemory_repo_root();
    let oid = blob_oid_at_head(&root, "Cargo.toml")
        .expect("blob_oid_at_head")
        .expect("Cargo.toml is tracked in the comemory repo");
    assert_eq!(
        oid.len(),
        40,
        "git blob OID should be 40 hex chars, got {oid:?}"
    );
    assert!(
        oid.chars().all(|c| c.is_ascii_hexdigit()),
        "non-hex chars in {oid}"
    );
}

#[test]
fn blob_oid_at_head_returns_none_for_bogus_path() {
    let root = comemory_repo_root();
    let got = blob_oid_at_head(&root, "does/not/exist/zzz.nope")
        .expect("blob_oid_at_head for bogus path");
    assert_eq!(got, None, "untracked path should yield None");
}

#[test]
fn blob_oid_at_head_returns_none_for_directory() {
    let root = comemory_repo_root();
    // `src` is a tree in the HEAD commit, not a blob.
    let got = blob_oid_at_head(&root, "src").expect("blob_oid_at_head for directory");
    assert_eq!(got, None, "a directory tree entry is not a blob");
}

#[test]
fn current_branch_returns_some_in_real_repo() {
    let root = comemory_repo_root();
    let branch = current_branch(&root).expect("current_branch");
    // CI may run on a detached HEAD tag checkout; tolerate that. When attached,
    // the shorthand must be a non-empty branch name.
    if let Some(name) = branch {
        assert!(!name.is_empty(), "branch shorthand should be non-empty");
    }
}

#[test]
fn blob_oid_at_head_none_when_head_is_unborn() {
    let tmp = TempDir::new().expect("tempdir");
    run_git(tmp.path(), &["init", "--quiet"]);
    // No commits: HEAD is unborn.
    let got = blob_oid_at_head(tmp.path(), "a.txt").expect("blob_oid_at_head on unborn HEAD");
    assert_eq!(got, None, "unborn HEAD has no committed blobs");
    assert_eq!(
        current_branch(tmp.path()).expect("current_branch on unborn HEAD"),
        None,
        "unborn HEAD has no resolvable branch"
    );
}

#[test]
fn blob_and_branch_track_committed_state_in_fixture() {
    let tmp = TempDir::new().expect("tempdir");
    make_repo_with_one_commit(&tmp);

    // Tracked file → Some(40-hex), matching the committed blob.
    let oid = blob_oid_at_head(tmp.path(), "a.txt")
        .expect("blob_oid_at_head")
        .expect("a.txt is committed");
    assert_eq!(oid.len(), 40, "got {oid:?}");

    // Untracked-on-disk file → None even though it exists in the working tree.
    std::fs::write(tmp.path().join("untracked.txt"), "x").expect("write untracked");
    assert_eq!(
        blob_oid_at_head(tmp.path(), "untracked.txt").expect("blob_oid_at_head untracked"),
        None,
        "untracked working-tree file has no HEAD blob"
    );

    // `make_repo_with_one_commit` pins the branch to `main`.
    assert_eq!(
        current_branch(tmp.path()).expect("current_branch"),
        Some("main".to_string())
    );
}

#[test]
fn current_branch_none_when_detached() {
    let tmp = TempDir::new().expect("tempdir");
    make_repo_with_one_commit(&tmp);
    let head = current_head(tmp.path()).expect("head");
    // Detach HEAD onto the commit OID directly.
    run_git(tmp.path(), &["checkout", "-q", &head]);
    assert_eq!(
        current_branch(tmp.path()).expect("current_branch detached"),
        None,
        "detached HEAD has no branch shorthand"
    );
    // A detached HEAD still resolves committed blobs.
    let oid = blob_oid_at_head(tmp.path(), "a.txt")
        .expect("blob_oid_at_head detached")
        .expect("a.txt still committed when detached");
    assert_eq!(oid.len(), 40, "got {oid:?}");
}

/// The exact `post-commit` body comemory shipped before the worktree-label
/// rule, as found installed in real repos. It carries the marker (so
/// `hook_installed` reports it) but labels every checkout by its OWN
/// basename, which is what filled the console with one "repository" per
/// `git worktree add`.
const LEGACY_HOOK_SCRIPT: &str = "#!/usr/bin/env bash\n\
     ROOT=\"$(git rev-parse --show-toplevel 2>/dev/null)\"\n\
     [ -z \"$ROOT\" ] && exit 0\n\
     REPO=\"$(basename \"$ROOT\")\"\n\
     ( comemory index-code --repo \"$REPO\" --path \"$ROOT\" >/dev/null 2>&1 & )\n\
     exit 0\n";

#[test]
fn hook_outdated_flags_a_comemory_hook_written_before_the_worktree_rule() {
    let tmp = TempDir::new().expect("tempdir");
    make_repo_with_one_commit(&tmp);
    let root = tmp.path();

    install_hook(root, "post-commit", LEGACY_HOOK_SCRIPT).expect("install legacy hook");
    assert!(
        hook_installed(root, "post-commit"),
        "the legacy body carries the marker, which is why nothing ever replaced it"
    );
    assert!(
        hook_outdated(root, "post-commit"),
        "a comemory hook whose body is not the shipped one is outdated"
    );

    install_hook(root, "post-commit", REINDEX_HOOK_SCRIPT).expect("install current hook");
    assert!(hook_installed(root, "post-commit"));
    assert!(
        !hook_outdated(root, "post-commit"),
        "the body this binary ships is never outdated"
    );
}

#[test]
fn hook_outdated_ignores_a_hook_comemory_did_not_write() {
    let tmp = TempDir::new().expect("tempdir");
    make_repo_with_one_commit(&tmp);
    let root = tmp.path();
    let hooks = hooks_dir(root);
    std::fs::create_dir_all(&hooks).expect("hooks dir");
    std::fs::write(hooks.join("post-commit"), "#!/bin/sh\necho mine\n").expect("foreign hook");

    assert!(!hook_installed(root, "post-commit"));
    assert!(
        !hook_outdated(root, "post-commit"),
        "somebody else's hook is foreign, never ours to call outdated"
    );
    assert!(
        !hook_outdated(root, "post-merge"),
        "a hook that does not exist is not outdated"
    );
}

#[test]
fn is_linked_worktree_separates_a_worktree_from_its_main_checkout() {
    let tmp = TempDir::new().expect("tempdir");
    let (main, wt) = main_and_linked_worktree(&tmp);
    let nested = wt.join("src").join("deep");
    std::fs::create_dir_all(&nested).expect("nested dir");

    let discovered =
        |p: &std::path::Path| git2::Repository::discover(p).map(|repo| is_linked_worktree(&repo));

    assert!(
        !discovered(&main).expect("discover main"),
        "the main checkout is the repository itself"
    );
    assert!(
        discovered(&wt).expect("discover worktree"),
        "a linked worktree is a second checkout, not a repository"
    );
    assert!(
        discovered(&nested).expect("discover from inside the worktree"),
        "the answer is the same from anywhere inside the worktree"
    );
    assert!(
        discovered(tmp.path()).is_err(),
        "outside any repository there is nothing to discover"
    );
}

/// `checkout_kind` answers about the directory it is HANDED, never an
/// ancestor — the rule the code-index push leans on to keep a worktree row,
/// and a row whose checkout is gone, off the wire.
#[test]
fn checkout_kind_never_walks_up_out_of_the_directory_it_was_given() {
    let tmp = TempDir::new().expect("tempdir");
    let (main, wt) = main_and_linked_worktree(&tmp);

    assert_eq!(checkout_kind(&main), CheckoutKind::Main);
    assert_eq!(checkout_kind(&wt), CheckoutKind::Linked);

    // A subdirectory of a checkout is not itself one: `discover` would walk
    // up and call this a repository.
    let inside = main.join("src");
    std::fs::create_dir_all(&inside).expect("nested dir");
    assert_eq!(checkout_kind(&inside), CheckoutKind::None);

    // What `git worktree remove` leaves when an ignored file survived it.
    std::fs::remove_dir_all(&wt).expect("remove the worktree");
    std::fs::create_dir_all(&wt).expect("leftover dir");
    std::fs::write(wt.join("build.log"), "left behind").expect("leftover file");
    assert_eq!(checkout_kind(&wt), CheckoutKind::None);

    std::fs::remove_dir_all(&wt).expect("remove the leftover");
    assert_eq!(
        checkout_kind(&wt),
        CheckoutKind::None,
        "a gone path is none"
    );
}
