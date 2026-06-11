//! Integration tests for `comemory::graph::cochange` against a REAL git
//! repo built with the git CLI — no mocked history. The fixture script:
//! commit1 touches `a.rs`+`b.rs`, commit2 touches `a.rs`+`b.rs`, commit3
//! touches `b.rs`+`c.rs`, commit4 is a 25-file mega-commit (must be
//! skipped). The base repo from `build_sample_repo` contributes a root
//! commit touching only `src.rs`, which exercises the root-commit (empty
//! parent tree) diff path and is filtered out by the known-files set.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use comemory::git_utils::current_head;
use comemory::graph::cochange::{mine_cochange, CoChange, MEGA_COMMIT_FILE_CAP};
use tempfile::TempDir;

#[path = "../common/git_setup.rs"]
mod git_setup;

/// Known-files set `{a.rs, b.rs, c.rs}` shared by every test.
fn known() -> HashSet<String> {
    ["a.rs", "b.rs", "c.rs"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// HEAD oid via the production `git_utils` helper, for cursor assertions.
fn head_oid_of(repo: &Path) -> String {
    current_head(repo).expect("resolve HEAD oid")
}

/// Build the four-commit fixture described in the module docs on top of
/// the shared one-commit sample repo. The mega-commit deliberately touches
/// `a.rs` and `c.rs` plus filler files: if the cap guard ever regressed,
/// a spurious `(a.rs, c.rs)` pair would surface in the assertions.
fn build_cochange_repo(root: &Path) -> PathBuf {
    let repo = git_setup::build_sample_repo(root);
    git_setup::commit_files(&repo, &[("a.rs", "a v1"), ("b.rs", "b v1")], "c1");
    git_setup::commit_files(&repo, &[("a.rs", "a v2"), ("b.rs", "b v2")], "c2");
    git_setup::commit_files(&repo, &[("b.rs", "b v3"), ("c.rs", "c v1")], "c3");
    // 2 known + (cap + 3) filler = 25 distinct files > MEGA_COMMIT_FILE_CAP.
    let filler: Vec<(String, String)> = (0..MEGA_COMMIT_FILE_CAP + 3)
        .map(|i| (format!("filler_{i}.rs"), format!("filler {i}")))
        .collect();
    let mut files: Vec<(&str, &str)> = vec![("a.rs", "a mega"), ("c.rs", "c mega")];
    files.extend(filler.iter().map(|(p, c)| (p.as_str(), c.as_str())));
    git_setup::commit_files(&repo, &files, "mega formatting sweep");
    repo
}

#[test]
fn first_run_mines_weighted_pairs_and_skips_mega_commit() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = build_cochange_repo(tmp.path());

    // `None` cursor: the walk covers the full (5-commit) history because
    // it is far below FIRST_RUN_COMMIT_LIMIT — the deepest pair-bearing
    // commit (c1) is only reachable if the bound did not cut the walk.
    let (pairs, cursor) = mine_cochange(&repo, &known(), None).expect("mine");
    assert_eq!(
        pairs,
        vec![
            CoChange {
                a: "a.rs".into(),
                b: "b.rs".into(),
                count: 2
            },
            CoChange {
                a: "b.rs".into(),
                b: "c.rs".into(),
                count: 1
            },
        ],
        "expected mega-commit skipped and pairs in lexicographic order"
    );
    assert_eq!(cursor, head_oid_of(&repo));
}

#[test]
fn incremental_mine_counts_only_commits_after_cursor() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = build_cochange_repo(tmp.path());
    let (_, cursor) = mine_cochange(&repo, &known(), None).expect("first mine");

    git_setup::commit_files(&repo, &[("a.rs", "a v5"), ("c.rs", "c v5")], "c5");
    let (pairs, new_cursor) =
        mine_cochange(&repo, &known(), Some(&cursor)).expect("incremental mine");
    assert_eq!(
        pairs,
        vec![CoChange {
            a: "a.rs".into(),
            b: "c.rs".into(),
            count: 1
        }]
    );
    assert_eq!(new_cursor, head_oid_of(&repo));
}

#[test]
fn mine_with_cursor_at_head_returns_empty_and_same_cursor() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = build_cochange_repo(tmp.path());
    let head = head_oid_of(&repo);

    let (pairs, cursor) = mine_cochange(&repo, &known(), Some(&head)).expect("mine at HEAD");
    assert!(pairs.is_empty(), "no commits newer than HEAD: {pairs:?}");
    assert_eq!(cursor, head);
}

#[test]
fn mine_on_repo_without_commits_is_an_error() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = tmp.path().join("empty-repo");
    git_setup::init_repo(&repo);

    let err = mine_cochange(&repo, &known(), None);
    assert!(
        err.is_err(),
        "unborn HEAD must surface as a best-effort error"
    );
}
