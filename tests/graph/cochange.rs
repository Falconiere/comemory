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

use crate::git_setup;

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

/// The full-history pair counts the fixture produces on a first run.
fn first_run_pairs() -> Vec<CoChange> {
    vec![
        CoChange {
            a: "a.rs".into(),
            b: "b.rs".into(),
            count: 2,
        },
        CoChange {
            a: "b.rs".into(),
            b: "c.rs".into(),
            count: 1,
        },
    ]
}

#[test]
fn first_run_mines_weighted_pairs_and_skips_mega_commit() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = build_cochange_repo(tmp.path());

    // `None` cursor: the walk covers the full (5-commit) history because
    // it is far below FIRST_RUN_COMMIT_LIMIT — the deepest pair-bearing
    // commit (c1) is only reachable if the bound did not cut the walk.
    let out = mine_cochange(&repo, &known(), None).expect("mine");
    assert_eq!(
        out.pairs,
        first_run_pairs(),
        "expected mega-commit skipped and pairs in lexicographic order"
    );
    assert_eq!(out.cursor, head_oid_of(&repo));
    assert!(!out.cursor_lost, "a None cursor is absent, not lost");
}

#[test]
fn incremental_mine_counts_only_commits_after_cursor() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = build_cochange_repo(tmp.path());
    let first = mine_cochange(&repo, &known(), None).expect("first mine");

    git_setup::commit_files(&repo, &[("a.rs", "a v5"), ("c.rs", "c v5")], "c5");
    let out = mine_cochange(&repo, &known(), Some(&first.cursor)).expect("incremental mine");
    assert_eq!(
        out.pairs,
        vec![CoChange {
            a: "a.rs".into(),
            b: "c.rs".into(),
            count: 1
        }]
    );
    assert_eq!(out.cursor, head_oid_of(&repo));
    assert!(!out.cursor_lost, "a resolvable cursor is not lost");
}

#[test]
fn mine_with_cursor_at_head_returns_empty_and_same_cursor() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = build_cochange_repo(tmp.path());
    let head = head_oid_of(&repo);

    let out = mine_cochange(&repo, &known(), Some(&head)).expect("mine at HEAD");
    assert!(
        out.pairs.is_empty(),
        "no commits newer than HEAD: {:?}",
        out.pairs
    );
    assert_eq!(out.cursor, head);
    assert!(!out.cursor_lost);
}

/// Resolved-cursor equivalence: hiding the cursor commit must produce
/// exactly the same counts as the old break-on-sight loop did for a
/// linear history — the deep cursor excludes itself and every ancestor,
/// leaving only the newer commits.
#[test]
fn deep_cursor_excludes_itself_and_all_ancestors() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = build_cochange_repo(tmp.path());
    // Cursor at c1 (HEAD~3): only c2 (a+b) and c3 (b+c) count — the mega
    // commit is skipped and c1 itself plus the root are hidden.
    let c1_parent_of_head = {
        let out = std::process::Command::new("git")
            .current_dir(&repo)
            .args(["rev-parse", "HEAD~3"])
            .output()
            .expect("rev-parse HEAD~3");
        assert!(out.status.success(), "rev-parse failed");
        String::from_utf8(out.stdout)
            .expect("utf8 oid")
            .trim()
            .to_string()
    };

    let out = mine_cochange(&repo, &known(), Some(&c1_parent_of_head)).expect("mine");
    assert_eq!(
        out.pairs,
        vec![
            CoChange {
                a: "a.rs".into(),
                b: "b.rs".into(),
                count: 1
            },
            CoChange {
                a: "b.rs".into(),
                b: "c.rs".into(),
                count: 1
            },
        ],
        "hidden cursor must match the old break-on-sight semantics"
    );
    assert!(!out.cursor_lost);
}

/// A cursor that no longer resolves to a commit (history rewrite + gc, or
/// a corrupted marker) must NOT walk uncapped to the root and silently
/// re-count: the pass degrades to a bounded first run and reports
/// `cursor_lost` so the caller resets the accumulated weights.
#[test]
fn lost_cursor_degrades_to_capped_first_run_and_signals_reset() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = build_cochange_repo(tmp.path());

    for garbage in [
        // Well-formed oid that names no object in this repo.
        "0123456789abcdef0123456789abcdef01234567",
        // Not an oid at all.
        "not-an-oid",
    ] {
        let out = mine_cochange(&repo, &known(), Some(garbage)).expect("mine with lost cursor");
        assert!(
            out.cursor_lost,
            "unresolvable cursor {garbage:?} must be reported lost"
        );
        assert_eq!(
            out.pairs,
            first_run_pairs(),
            "lost cursor must re-mine exactly the bounded first-run history"
        );
        assert_eq!(out.cursor, head_oid_of(&repo));
    }
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
