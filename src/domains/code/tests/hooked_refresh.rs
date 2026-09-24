#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `hooked_refresh` over real repositories: real `git` commits, hooks written
//! by the real install core, indexes built by the real `index_code::run`.
//! Every repo sets `core.hooksPath=/dev/null` right after `git init`: the
//! case under test is a HEAD move no hook saw, and it keeps an installed hook
//! (fired by a commit or a `git worktree add`) from launching whatever
//! `comemory` is on this host's `PATH` against its real data directory.
//! `hooks_dir` reads the common dir, not `core.hooksPath`, so the hooks still
//! count as installed.

use std::path::{Path, PathBuf};

use comemory::config::{Config, Paths};
use comemory::domains::code::hooked_refresh::{
    Checkout, RefreshStats, index_checkout, refresh_stale, resolve_checkout, swept_by_a_later_pass,
};
use comemory::domains::code::index_code::{self, IndexMode};
use comemory::domains::code::install_hooks;
use comemory::store::{Connection, connection, repo_marker};
use comemory::utilities::context::Ctx;

use crate::test_common::git_commit::commit_files;
use crate::test_common::git_repo::{init_repo, run_git};
use crate::test_common::git_worktree::add_worktree;

struct Rig {
    home: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    conn: Connection,
}

fn rig() -> Rig {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("data"));
    paths.ensure_dirs().unwrap();
    let conn = connection::open(paths.db_path()).unwrap();
    Rig {
        home,
        paths,
        cfg: Config::defaults(),
        conn,
    }
}

impl Rig {
    /// A real repo `<home>/<name>` with one committed Rust file, indexed
    /// under `name`, with comemory's hooks installed when `hooked`.
    fn repo(&mut self, name: &str, hooked: bool) -> PathBuf {
        let root = self.home.path().join(name);
        init_repo(&root);
        run_git(&root, &["config", "core.hooksPath", "/dev/null"]);
        commit_files(
            &root,
            &[("src/lib.rs", "pub fn one() -> u8 { 1 }\n")],
            "init",
        );
        if hooked {
            let mut ctx = Ctx::lazy(&self.paths, &self.cfg);
            install_hooks::run(
                &mut ctx,
                install_hooks::Request {
                    repo: root.display().to_string(),
                    force: false,
                },
            )
            .unwrap();
        }
        self.index(name, &root);
        root.canonicalize().unwrap()
    }

    fn index(&mut self, label: &str, root: &Path) {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        index_code::run(
            &mut ctx,
            index_code::Request {
                repo: label.to_string(),
                path: root.display().to_string(),
                mode: IndexMode::Incremental,
            },
        )
        .unwrap();
    }

    fn sweep(&mut self, skip: Option<&str>) -> RefreshStats {
        let mut stats = RefreshStats::default();
        refresh_stale(&self.paths, &self.cfg, &mut self.conn, skip, &mut stats).unwrap();
        stats
    }

    fn last_head(&self, label: &str) -> Option<String> {
        repo_marker::last_head(&self.conn, label).unwrap()
    }
}

/// A commit no hook sees: a rebase with hooks off, a GUI client that ran none.
fn commit_without_hooks(root: &Path, file: &str, body: &str) -> String {
    std::fs::write(root.join(file), body).unwrap();
    run_git(root, &["add", "-A"]);
    run_git(root, &["commit", "-q", "-m", "hookless"]);
    comemory::domains::code::git_utils::current_head(root).unwrap()
}

#[test]
fn a_stale_hooked_repo_is_refreshed_and_a_fresh_one_is_left_alone() {
    let mut rig = rig();
    let stale = rig.repo("stale", true);
    rig.repo("fresh", true);
    let fresh_head = rig.last_head("fresh");
    let new_head = commit_without_hooks(&stale, "src/two.rs", "pub fn two() -> u8 { 2 }\n");
    assert_ne!(rig.last_head("stale").as_deref(), Some(new_head.as_str()));

    let stats = rig.sweep(None);

    assert_eq!(
        stats,
        RefreshStats {
            checked: 2,
            refreshed: 1,
            failed: 0,
            errors: vec![],
        }
    );
    assert_eq!(rig.last_head("stale").as_deref(), Some(new_head.as_str()));
    assert_eq!(rig.last_head("fresh"), fresh_head);

    // Nothing moved since: a second pass re-indexes nothing.
    assert_eq!(rig.sweep(None).refreshed, 0);
}

#[test]
fn unhooked_archived_vanished_and_linked_worktree_rows_are_never_refreshed() {
    let mut rig = rig();
    let unhooked = rig.repo("unhooked", false);
    let archived = rig.repo("archived", true);
    let vanished = rig.repo("vanished", true);
    let main = rig.repo("parent", true);
    for root in [&unhooked, &archived, &vanished, &main] {
        commit_without_hooks(root, "src/two.rs", "pub fn two() -> u8 { 2 }\n");
    }
    repo_marker::set_archived(&rig.conn, "archived", true).unwrap();
    std::fs::remove_dir_all(&vanished).unwrap();
    // A row whose recorded root is a linked worktree, the way a pre-rule hook
    // minted one: connected under its own label, then indexed.
    let wt = rig.home.path().join("parent-wt");
    add_worktree(&main, &wt, "wt-branch");
    let mut ctx = Ctx::borrowed(&rig.paths, &rig.cfg, &mut rig.conn);
    comemory::domains::code::repo_admin::connect(
        &mut ctx,
        comemory::domains::code::repo_admin::ConnectRequest {
            root: wt.display().to_string(),
            repo: Some("parent-wt".to_string()),
            index_now: false,
        },
    )
    .unwrap();
    rig.index("parent-wt", &wt);
    commit_without_hooks(&wt, "src/three.rs", "pub fn three() -> u8 { 3 }\n");
    let before: Vec<Option<String>> = ["unhooked", "archived", "vanished", "parent-wt"]
        .iter()
        .map(|l| rig.last_head(l))
        .collect();

    // `parent` itself is excluded with `skip`, so every other row must be a
    // skip reason rather than a refresh.
    let stats = rig.sweep(Some("parent"));

    assert_eq!(stats, RefreshStats::default(), "nothing was eligible");
    let after: Vec<Option<String>> = ["unhooked", "archived", "vanished", "parent-wt"]
        .iter()
        .map(|l| rig.last_head(l))
        .collect();
    assert_eq!(before, after);
    assert!(
        repo_marker::all_repos(&rig.conn)
            .unwrap()
            .contains(&"vanished".to_string()),
        "a vanished root keeps its marker — an unmounted volume must come back"
    );
}

#[test]
fn a_linked_worktree_checkout_indexes_under_the_main_label() {
    let mut rig = rig();
    let main = rig.repo("parent", true);
    let wt = rig.home.path().join("feature-wt");
    add_worktree(&main, &wt, "feature");
    let head = commit_without_hooks(&wt, "src/feat.rs", "pub fn feat() -> u8 { 4 }\n");

    let checkout = resolve_checkout(&wt).expect("a worktree resolves");
    assert_eq!(checkout.label, "parent");
    assert_eq!(checkout.root, wt.canonicalize().unwrap());
    let mut stats = RefreshStats::default();
    index_checkout(&rig.paths, &rig.cfg, &mut rig.conn, &checkout, &mut stats).unwrap();

    assert_eq!((stats.checked, stats.refreshed, stats.failed), (1, 1, 0));
    assert_eq!(rig.last_head("parent").as_deref(), Some(head.as_str()));
    assert_eq!(
        repo_marker::all_repos(&rig.conn).unwrap(),
        vec!["parent".to_string()],
        "no row is minted for the worktree directory"
    );
}

#[test]
fn a_path_outside_any_work_tree_resolves_to_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(resolve_checkout(dir.path()), None);
}

#[test]
fn only_a_registered_unarchived_same_root_checkout_is_swept_by_a_later_pass() {
    let mut rig = rig();
    let main = rig.repo("parent", true);
    let wt = rig.home.path().join("side-wt");
    add_worktree(&main, &wt, "side");
    let fresh = rig.home.path().join("fresh");
    init_repo(&fresh);
    commit_files(&fresh, &[("src/lib.rs", "pub fn f() {}\n")], "init");

    let registered = resolve_checkout(&main).unwrap();
    assert!(swept_by_a_later_pass(&rig.conn, &registered).unwrap());
    let worktree = resolve_checkout(&wt).unwrap();
    assert!(
        !swept_by_a_later_pass(&rig.conn, &worktree).unwrap(),
        "the sweep walks the recorded main root, never this worktree"
    );
    let unregistered = resolve_checkout(&fresh).unwrap();
    assert!(!swept_by_a_later_pass(&rig.conn, &unregistered).unwrap());
    repo_marker::set_archived(&rig.conn, "parent", true).unwrap();
    assert!(!swept_by_a_later_pass(&rig.conn, &registered).unwrap());
}

#[test]
fn an_archived_checkout_is_not_indexed_by_step_one() {
    let mut rig = rig();
    let root = rig.repo("parent", true);
    let before = rig.last_head("parent");
    commit_without_hooks(&root, "src/two.rs", "pub fn two() -> u8 { 2 }\n");
    repo_marker::set_archived(&rig.conn, "parent", true).unwrap();

    let mut stats = RefreshStats::default();
    let checkout = Checkout {
        label: "parent".to_string(),
        root,
    };
    index_checkout(&rig.paths, &rig.cfg, &mut rig.conn, &checkout, &mut stats).unwrap();

    assert_eq!(stats, RefreshStats::default());
    assert_eq!(rig.last_head("parent"), before);
}
