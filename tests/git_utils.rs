//! Integration tests for `qwick_memory::git_utils`. We shell out to the real `git`
//! binary to construct fixtures (rather than driving `git2` directly) so the
//! test exercises the same on-disk layout a user repo would have — including
//! the `.git/` directory `Repository::discover` looks for.

use std::process::Command;

use qwick_memory::git_utils::{changed_files, current_head, install_hook};
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
