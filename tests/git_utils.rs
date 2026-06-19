//! Integration tests for `comemory::git_utils`. We shell out to the real `git`
//! binary to construct fixtures (rather than driving `git2` directly) so the
//! test exercises the same on-disk layout a user repo would have — including
//! the `.git/` directory `Repository::discover` looks for.

use std::process::Command;

use comemory::git_utils::{
    blob_oid_at_head, changed_files, current_branch, current_head, install_hook,
};
use tempfile::TempDir;

/// Build a git repo in `dir` with a single commit. Returns the path so the
/// caller can keep the `TempDir` alive. Panics on any git failure because the
/// test environment is broken if `git init`/`git commit` can't succeed.
fn make_repo_with_one_commit(dir: &TempDir) {
    let p = dir.path();
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
