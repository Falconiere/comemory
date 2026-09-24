#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory sync --action auto` and the hooks that fire it, driven as real
//! processes: the built binary, real `git` commits running the real installed
//! hooks, from a cwd outside every repo.
//!
//! Each test sandboxes `HOME` and runs both git and comemory with
//! `PATH=/usr/bin:/bin`, so the only `comemory` a hook can find is the built
//! binary linked at `$HOME/.cargo/bin/comemory` — one of the hook's fallback
//! locations — and the host's own install is never launched (nor its data
//! directory touched).

#[path = "common/git_repo.rs"]
mod git_repo;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::cargo::CommandCargoExt as _;
use comemory::utilities::file_lock::FileLock;
use tempfile::TempDir;

const BARE_PATH: &str = "/usr/bin:/bin";

struct Sandbox {
    home: TempDir,
}

impl Sandbox {
    /// A temp `HOME` whose `.cargo/bin/comemory` is the built binary.
    fn new() -> Self {
        let home = TempDir::new().unwrap();
        let bin = home.path().join(".cargo").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let built = Command::cargo_bin("comemory").unwrap();
        std::os::unix::fs::symlink(built.get_program(), bin.join("comemory")).unwrap();
        std::fs::create_dir_all(home.path().join("elsewhere")).unwrap();
        Self { home }
    }

    fn data(&self) -> PathBuf {
        self.home.path().join("data")
    }

    /// The built binary, run from a directory outside every repo.
    fn comemory(&self, args: &[&str]) -> Command {
        let mut cmd = Command::cargo_bin("comemory").unwrap();
        cmd.args(args)
            .current_dir(self.home.path().join("elsewhere"))
            .env("HOME", self.home.path())
            .env("PATH", BARE_PATH)
            .env("COMEMORY_DATA_DIR", self.data());
        cmd
    }

    fn run_json(&self, args: &[&str]) -> serde_json::Value {
        let out = self.comemory(args).output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        serde_json::from_slice(&out.stdout).unwrap()
    }

    /// `git` with the sandbox `HOME` and the bare `PATH`, so a hook it runs
    /// finds comemory only through the fallback.
    fn git(&self, repo: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo)
            .env("HOME", self.home.path())
            .env("PATH", BARE_PATH)
            .env("COMEMORY_DATA_DIR", self.data())
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", stderr(&out));
    }

    /// A real repo `<home>/<name>` with one committed file, hooks not yet
    /// installed.
    fn repo(&self, name: &str) -> PathBuf {
        let root = self.home.path().join(name);
        git_repo::init_repo(&root);
        self.commit(&root, "src/lib.rs", "pub fn one() -> u8 { 1 }\n");
        root.canonicalize().unwrap()
    }

    /// Write one file and commit it through git, hooks included.
    fn commit(&self, repo: &Path, file: &str, body: &str) -> String {
        let path = repo.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        self.git(repo, &["add", "-A"]);
        self.git(repo, &["commit", "-q", "-m", file]);
        head(repo)
    }

    /// The same commit with every hook switched off: a HEAD move no hook saw.
    fn commit_hookless(&self, repo: &Path, file: &str, body: &str) -> String {
        std::fs::write(repo.join(file), body).unwrap();
        self.git(repo, &["add", "-A"]);
        self.git(
            repo,
            &["-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", file],
        );
        head(repo)
    }

    fn install_hooks(&self, repo: &Path) -> serde_json::Value {
        self.run_json(&["install-hooks", "--repo", repo.to_str().unwrap(), "--json"])
    }

    fn repo_row(&self, label: &str) -> Option<serde_json::Value> {
        self.run_json(&["repos", "--json"])["repos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["repo"] == label)
            .cloned()
    }

    /// Poll `repos --json` until `label`'s `last_head` is `head` (any head
    /// when `None`); the hook's pass runs detached.
    fn wait_for_head(&self, label: &str, head: Option<&str>) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(row) = self.repo_row(label)
                && row["last_head"]
                    .as_str()
                    .is_some_and(|h| head.is_none_or(|want| want == h))
            {
                return row;
            }
            assert!(
                Instant::now() < deadline,
                "{label} never reached {head:?}: {:?}",
                self.repo_row(label)
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Block until no pass holds the sync lock, so a test starts quiet.
    fn settle(&self) {
        drop(FileLock::acquire(&self.data().join("sync.lock"), "test").unwrap());
    }
}

fn head(repo: &Path) -> String {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// AC-8: a never-indexed repo is registered by `install-hooks` alone.
#[test]
fn install_hooks_kicks_the_first_pass_for_a_never_indexed_repo() {
    let sb = Sandbox::new();
    let repo = sb.repo("kicked");

    let report = sb.install_hooks(&repo);

    assert_eq!(report["kicked"], serde_json::json!(true), "{report}");
    assert_eq!(report["installed"].as_array().unwrap().len(), 4);
    sb.wait_for_head("kicked", Some(&head(&repo)));
}

/// AC-1: a HEAD move no hook saw is caught by a pass run from anywhere.
#[test]
fn an_auto_pass_from_an_unrelated_cwd_refreshes_a_stale_hooked_repo() {
    let sb = Sandbox::new();
    let repo = sb.repo("stale");
    sb.install_hooks(&repo);
    sb.wait_for_head("stale", None);
    sb.settle();
    let new_head = sb.commit_hookless(&repo, "src/two.rs", "pub fn two() -> u8 { 2 }\n");
    assert_eq!(sb.repo_row("stale").unwrap()["status"], "stale");

    let report = sb.run_json(&["sync", "--action", "auto", "--json"]);

    assert_eq!(report["coalesced"], serde_json::json!(false), "{report}");
    assert_eq!(report["logged_in"], serde_json::json!(false));
    assert_eq!(
        report["refresh"]["refreshed"],
        serde_json::json!(1),
        "{report}"
    );
    assert!(report.get("pull").is_none(), "no network leg logged out");
    let row = sb.repo_row("stale").unwrap();
    assert_eq!(row["last_head"], serde_json::json!(new_head));
    assert_eq!(row["status"], "fresh");
}

/// AC-7: a commit whose `PATH` has no comemory still reaches the index.
#[test]
fn a_commit_with_no_comemory_on_path_is_indexed_through_the_fallback() {
    let sb = Sandbox::new();
    let repo = sb.repo("gui");
    sb.install_hooks(&repo);
    sb.wait_for_head("gui", None);

    let new_head = sb.commit(&repo, "src/two.rs", "pub fn two() -> u8 { 2 }\n");

    sb.wait_for_head("gui", Some(&new_head));
}

/// `post-rewrite`, the hook this release adds, runs the pass on its own: with
/// the other three disabled, an amend (a rewrite) still reaches the index.
#[test]
fn an_amend_reaches_the_index_through_post_rewrite_alone() {
    let sb = Sandbox::new();
    let repo = sb.repo("rewritten");
    sb.install_hooks(&repo);
    sb.wait_for_head("rewritten", None);
    sb.settle();
    for hook in ["post-commit", "post-merge", "post-checkout"] {
        sb.run_json(&[
            "hooks",
            "--repo",
            repo.to_str().unwrap(),
            "--disable",
            hook,
            "--json",
        ]);
    }
    std::fs::write(repo.join("src/lib.rs"), "pub fn one() -> u8 { 11 }\n").unwrap();
    sb.git(&repo, &["add", "-A"]);

    sb.git(&repo, &["commit", "-q", "--amend", "-m", "amended"]);

    sb.wait_for_head("rewritten", Some(&head(&repo)));
}

/// AC-12: a commit in a linked worktree indexes under the main label.
#[test]
fn a_commit_in_a_linked_worktree_indexes_under_the_main_label() {
    let sb = Sandbox::new();
    let main = sb.repo("parent");
    sb.install_hooks(&main);
    sb.wait_for_head("parent", None);
    let wt = sb.home.path().join("parent-feature");
    sb.git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            wt.to_str().unwrap(),
        ],
    );

    let new_head = sb.commit(&wt, "src/feature.rs", "pub fn feature() {}\n");

    sb.wait_for_head("parent", Some(&new_head));
    sb.settle();
    assert!(
        sb.repo_row("parent-feature").is_none(),
        "no repository is minted for the worktree directory"
    );
}

/// AC-5: with a pass running, of three more triggers exactly one queues and
/// the other two coalesce at once; the queued one runs after the release.
#[test]
fn triggers_behind_a_running_pass_coalesce_into_one_queued_pass() {
    let sb = Sandbox::new();
    let repo = sb.repo("busy");
    sb.install_hooks(&repo);
    sb.wait_for_head("busy", None);
    sb.settle();
    let running = FileLock::acquire(&sb.data().join("sync.lock"), "test").unwrap();

    let mut children: Vec<Child> = (0..3)
        .map(|_| {
            sb.comemory(&["sync", "--action", "auto", "--json"])
                .stdout(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut exited = Vec::new();
    while exited.len() < 2 {
        assert!(Instant::now() < deadline, "two triggers never coalesced");
        let mut i = 0;
        while i < children.len() {
            if children[i].try_wait().unwrap().is_some() {
                exited.push(children.remove(i).wait_with_output().unwrap());
            } else {
                i += 1;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    for out in &exited {
        assert!(out.status.success(), "{}", stderr(out));
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v, serde_json::json!({"action": "auto", "coalesced": true}));
    }
    assert_eq!(children.len(), 1);
    assert!(
        children[0].try_wait().unwrap().is_none(),
        "the queued pass must still be waiting on the running one"
    );

    drop(running);
    let out = children.remove(0).wait_with_output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["coalesced"], serde_json::json!(false), "{v}");
}

#[test]
fn path_is_refused_with_any_action_but_auto() {
    let sb = Sandbox::new();
    let out = sb
        .comemory(&["sync", "--action", "push", "--path", "."])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    assert!(stderr(&out).contains("--path is only accepted with --action auto"));
}

#[test]
fn an_auto_pass_prints_nothing_without_json() {
    let sb = Sandbox::new();
    let out = sb.comemory(&["sync", "--action", "auto"]).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        out.stdout.is_empty(),
        "{:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}
