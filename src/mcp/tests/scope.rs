#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/mcp/scope.rs`, against real git checkouts built with
//! the real `git` binary: a main worktree and a `git worktree add` linked one
//! must yield ONE label — the main worktree's basename — so a session started
//! inside a linked checkout files memories under the repo it belongs to.

use comemory::mcp::scope;
use tempfile::tempdir;

use crate::test_common::{git_commit, git_repo, git_worktree};

#[test]
fn a_linked_worktree_yields_the_main_worktree_basename() {
    let dir = tempdir().expect("tempdir");
    let main = dir.path().join("sample");
    git_repo::init_repo(&main);
    git_commit::commit_files(&main, &[("README.md", "sample\n")], "init");

    let linked = dir.path().join("sample-feature");
    git_worktree::add_worktree(&main, &linked, "feature");

    assert_eq!(
        scope::default_repo(None, &main).as_deref(),
        Some("sample"),
        "the main worktree files under its own basename"
    );
    assert_eq!(
        scope::default_repo(None, &linked).as_deref(),
        Some("sample"),
        "a linked worktree shares the main worktree's label, not its own dir name"
    );
}

#[test]
fn a_directory_outside_any_repo_has_no_default_scope() {
    let dir = tempdir().expect("tempdir");
    assert_eq!(scope::default_repo(None, dir.path()), None);
}

#[test]
fn an_explicit_flag_wins_over_the_working_directory() {
    let dir = tempdir().expect("tempdir");
    let main = dir.path().join("sample");
    git_repo::init_repo(&main);
    git_commit::commit_files(&main, &[("README.md", "sample\n")], "init");

    assert_eq!(
        scope::default_repo(Some("pinned".into()), &main).as_deref(),
        Some("pinned"),
        "--repo overrides the discovered label"
    );
    // An empty flag is no opinion, not an unnamed scope.
    assert_eq!(
        scope::default_repo(Some("   ".into()), &main).as_deref(),
        Some("sample"),
        "a blank --repo falls through to the working directory"
    );
    assert_eq!(scope::default_repo(Some(String::new()), dir.path()), None);
}

#[test]
fn resolve_prefers_an_explicit_parameter_and_treats_empty_as_absent() {
    assert_eq!(
        scope::resolve(Some("a".into()), Some("b")).as_deref(),
        Some("a"),
        "an explicit parameter wins"
    );
    assert_eq!(
        scope::resolve(None, Some("b")).as_deref(),
        Some("b"),
        "the session default fills in"
    );
    assert_eq!(
        scope::resolve(Some(String::new()), Some("b")).as_deref(),
        Some("b"),
        "an empty parameter counts as absent"
    );
    assert_eq!(
        scope::resolve(Some(" \t ".into()), None),
        None,
        "whitespace-only with no default resolves to no scope"
    );
    assert_eq!(scope::resolve(None, None), None);
}
