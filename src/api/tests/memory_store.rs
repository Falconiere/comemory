#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for `src/api/memory_store.rs` against REAL data: memories saved
//! through `api::save::run`, real `git init`ed work trees (the shared
//! `tests/common/git_repo.rs` fixture), and a real bare repo standing in for
//! the remote. Nothing here is mocked — the `store-sync` job's whole value is
//! that it drives the actual `git` binary.
//!
//! The HTTP half (envelopes, the job lifecycle, the `501`) lives in
//! `src/serve/routes/tests/memory_stores.rs`.

use std::cell::RefCell;
use std::path::Path;

use comemory::api::{self, Ctx, memory_store};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::store::connection;
use tempfile::TempDir;

use crate::test_common::git_repo::{init_repo, run_git};

/// A bare repo with `main` as its initial branch, standing in for the remote.
fn bare_remote() -> TempDir {
    let bare = TempDir::new().expect("bare tempdir");
    run_git(
        bare.path(),
        &["init", "-q", "--bare", "--initial-branch=main"],
    );
    bare
}

/// Turn `home` into a git work tree with one root commit whose `main` tracks
/// `origin` = `bare` — the shape a store that has synced before has.
fn track_origin(home: &Path, bare: &Path) {
    init_repo(home);
    run_git(home, &["commit", "-q", "--allow-empty", "-m", "init"]);
    run_git(home, &["remote", "add", "origin", &bare.to_string_lossy()]);
    run_git(home, &["push", "-q", "-u", "origin", "main"]);
}

/// A second clone of `bare` with its own local identity — "another
/// machine" that pushes to the same remote.
fn clone_of(bare: &Path) -> TempDir {
    let clone = TempDir::new().expect("clone tempdir");
    run_git(clone.path(), &["clone", "-q", &bare.to_string_lossy(), "."]);
    run_git(clone.path(), &["config", "user.email", "other@example.com"]);
    run_git(clone.path(), &["config", "user.name", "other"]);
    clone
}

/// `git add -A && git commit` in `repo`, signing disabled so a developer's
/// global `commit.gpgsign` cannot reach for a pinentry mid-test.
fn commit_all(repo: &Path, msg: &str) {
    run_git(repo, &["add", "-A"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-q", "-m", msg],
    );
}

/// Captured stdout of one `git` read in `repo`, for asserting on state.
fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .expect("invoke git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// File name of the single seeded memory under `<home>/memories`.
fn only_memory(home: &Path) -> String {
    let mut names: Vec<String> = std::fs::read_dir(home.join("memories"))
        .expect("read memories dir")
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "exactly one seeded memory: {names:?}");
    names.pop().expect("one name")
}

/// Run the sync with `push`, capturing the log lines it emits.
fn run_sync(
    home: &Path,
    cfg: &Config,
    push: Option<bool>,
) -> (
    comemory::errors::Result<memory_store::SyncResponse>,
    Vec<String>,
) {
    let paths = Paths::new(home);
    let mut ctx = Ctx::lazy(&paths, cfg);
    let sink = RefCell::new(Vec::<String>::new());
    let result = memory_store::sync(
        &mut ctx,
        memory_store::STORE_ID,
        &memory_store::SyncRequest { push },
        |line| sink.borrow_mut().push(line.to_string()),
    );
    (result, sink.into_inner())
}

/// A data dir with `memories/` + `.trash/` created, and `n` real memories
/// saved through the same path `comemory save` takes.
fn seeded(n: usize) -> TempDir {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).expect("open db");
    for i in 0..n {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let req = api::save::Request {
            body: format!("memory store fixture body number {i}"),
            title: None,
            kind: Kind::Note,
            repo: "demo".to_string(),
            tags: Vec::new(),
            author: String::new(),
            quality: 3,
            supersedes: Vec::new(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        };
        api::save::run(&mut ctx, req, false, None).expect("seed save");
    }
    home
}

/// Count `*.md` files directly under `<home>/memories`, the on-disk truth
/// `Store::markdown_files` is asserted against.
fn on_disk_markdown(home: &TempDir) -> u64 {
    std::fs::read_dir(home.path().join("memories"))
        .expect("read memories dir")
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .count() as u64
}

#[test]
fn list_reports_one_store_matching_the_on_disk_markdown_count() {
    let home = seeded(2);
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let stores = memory_store::list(&mut ctx).expect("list");

    assert_eq!(stores.len(), 1, "comemory models exactly one memory store");
    let store = &stores[0];
    assert_eq!(store.id, memory_store::STORE_ID);
    assert_eq!(store.path, paths.memories_dir().to_string_lossy());
    assert_eq!(store.markdown_files, on_disk_markdown(&home));
    assert_eq!(store.markdown_files, 2);
    assert_eq!(store.trashed_files, 0);
    assert!(!store.push_on_save, "[git] auto_sync defaults to false");
    assert!(!store.sync.is_git_repo, "a bare temp dir is not a git repo");
    assert_eq!(store.sync.dirty, None);
    assert_eq!(store.sync.ahead, None);
    assert_eq!(store.sync.behind, None);
}

/// The read side must not create `comemory.db` as a side effect (the
/// must-not-create-the-db invariant `api::stats` documents).
#[test]
fn list_never_creates_the_database() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let stores = memory_store::list(&mut ctx).expect("list on an empty data dir");

    assert_eq!(stores[0].markdown_files, 0);
    assert!(
        !paths.db_path().exists(),
        "listing the store must not create {}",
        paths.db_path().display()
    );
}

#[test]
fn get_reports_the_branch_and_dirty_count_of_a_real_repo() {
    let home = seeded(1);
    init_repo(home.path());
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    // A freshly `git init`ed repo has an unborn HEAD: there is no commit for
    // a branch name to point at yet, which is a state, not a failure.
    let unborn = memory_store::get(&mut ctx, memory_store::STORE_ID).expect("get on unborn HEAD");
    assert!(unborn.sync.is_git_repo);
    assert_eq!(unborn.sync.branch, None, "unborn HEAD has no branch");

    run_git(
        home.path(),
        &["commit", "-q", "--allow-empty", "-m", "init"],
    );
    let store = memory_store::get(&mut ctx, memory_store::STORE_ID).expect("get");

    assert!(store.sync.is_git_repo);
    assert_eq!(store.sync.branch.as_deref(), Some("main"));
    assert_eq!(
        store.sync.dirty,
        Some(1),
        "the one saved memory is untracked under memories/"
    );
    assert_eq!(store.sync.ahead, None, "no upstream configured yet");
    assert_eq!(store.remote, None, "no origin remote configured yet");
}

#[test]
fn get_rejects_any_other_id_as_not_found() {
    let home = seeded(0);
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let err = memory_store::get(&mut ctx, "second").expect_err("unknown id must be NotFound");

    assert!(
        matches!(err, comemory::errors::Error::NotFound(_)),
        "expected NotFound, got {err:?}"
    );
    assert!(err.to_string().contains("second"), "message: {err}");
}

/// The patch writes only the keys it was given, and the file it produces is a
/// config the real loader accepts — asserted by loading it back through
/// `Config::defaults().with_file(..)`, the same path every command takes.
#[test]
fn patch_writes_only_the_supplied_git_keys_and_reloads_cleanly() {
    let home = seeded(0);
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let store = memory_store::patch(
        &mut ctx,
        memory_store::STORE_ID,
        &memory_store::PatchRequest {
            push_on_save: Some(true),
            remote: None,
        },
    )
    .expect("patch push_on_save");

    assert!(store.push_on_save, "the response reflects the write");
    let text = std::fs::read_to_string(paths.config_file()).expect("config.toml");
    assert!(text.contains("[git]"), "config.toml:\n{text}");
    assert!(text.contains("auto_sync = true"), "config.toml:\n{text}");
    assert!(
        !text.contains("remote"),
        "an absent key must not be written:\n{text}"
    );
    let reloaded = Config::defaults()
        .with_file(paths.config_file().as_path())
        .expect("the patched config.toml must load");
    assert!(reloaded.git.auto_sync);

    // Patching the other key leaves the first one alone.
    let mut ctx = Ctx::lazy(&paths, &reloaded);
    let store = memory_store::patch(
        &mut ctx,
        memory_store::STORE_ID,
        &memory_store::PatchRequest {
            push_on_save: None,
            remote: Some("backup".to_string()),
        },
    )
    .expect("patch remote");

    assert_eq!(store.remote.as_deref(), Some("backup"));
    assert!(store.push_on_save, "the untouched key keeps its value");
    let reloaded = Config::defaults()
        .with_file(paths.config_file().as_path())
        .expect("reload");
    assert!(reloaded.git.auto_sync);
    assert_eq!(reloaded.git.remote, "backup");
}

#[test]
fn patch_rejects_any_other_id_before_touching_the_file() {
    let home = seeded(0);
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let err = memory_store::patch(
        &mut ctx,
        "second",
        &memory_store::PatchRequest {
            push_on_save: Some(true),
            remote: None,
        },
    )
    .expect_err("unknown id must be NotFound");

    assert!(
        matches!(err, comemory::errors::Error::NotFound(_)),
        "expected NotFound, got {err:?}"
    );
    assert!(!paths.config_file().exists(), "no file may be written");
}

#[test]
fn create_is_always_unsupported() {
    let err = memory_store::create(memory_store::CreateRequest {
        path: "/tmp/another-store".to_string(),
        remote: None,
        push_on_save: Some(true),
    })
    .expect_err("a second store must be refused");

    assert!(
        matches!(err, comemory::errors::Error::Unsupported(_)),
        "expected Unsupported, got {err:?}"
    );
}

#[test]
fn sync_on_a_non_git_data_dir_is_a_bad_request() {
    let home = seeded(1);
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let err = memory_store::sync(
        &mut ctx,
        memory_store::STORE_ID,
        &memory_store::SyncRequest::default(),
        |_line| {},
    )
    .expect_err("a data dir outside any repo cannot be synced");

    assert!(
        matches!(err, comemory::errors::Error::BadRequest(_)),
        "expected BadRequest, got {err:?}"
    );
    assert_eq!(
        err.to_string(),
        "bad request: memory store is not a git repository"
    );
}

#[test]
fn sync_commits_and_pushes_the_memories_dir_to_a_real_remote() {
    let home = seeded(1);
    let bare = bare_remote();
    track_origin(home.path(), bare.path());
    let before = remote_head(&bare);

    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let (done, log) = run_sync(home.path(), &cfg, Some(true));
    let done = done.expect("sync");

    assert!(done.pulled, "an upstream exists, so pull runs");
    assert!(done.committed, "the seeded memory is a new file");
    assert!(done.pushed);
    assert!(done.conflicts.is_empty());
    let commit = done.commit.expect("commit oid");
    assert_eq!(commit.len(), 40, "commit: {commit}");
    assert_eq!(remote_head(&bare), commit, "the remote must have advanced");
    assert_ne!(remote_head(&bare), before);
    assert!(
        log.iter().any(|l| l.starts_with("git commit")),
        "every git step is logged: {log:?}"
    );

    // A second run has nothing to stage: a successful no-op, not an error.
    let again = memory_store::sync(
        &mut ctx,
        memory_store::STORE_ID,
        &memory_store::SyncRequest { push: Some(false) },
        |_line| {},
    )
    .expect("second sync");
    assert!(!again.committed, "nothing changed since the first sync");
    assert!(!again.pushed);
    assert_eq!(again.commit, None);
}

/// HEAD of the bare "remote" repo, read with `--git-dir` (it has no work
/// tree of its own).
fn remote_head(bare: &TempDir) -> String {
    let out = std::process::Command::new("git")
        .args([
            "--git-dir",
            &bare.path().to_string_lossy(),
            "rev-parse",
            "HEAD",
        ])
        .output()
        .expect("invoke git rev-parse");
    assert!(out.status.success(), "git rev-parse in the bare remote");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Why the sync commits BEFORE it pulls: `--autostash` stashes tracked
/// changes only, so an untracked new memory whose path also arrives from
/// upstream used to fail the pull with "untracked working tree files would
/// be overwritten" — no `CONFLICT` marker, a generic error on every retry.
/// Committed first, an identical upstream copy is a clean rebase (git skips
/// the already-applied commit) and the run succeeds.
#[test]
fn sync_commits_before_pulling_so_an_identical_upstream_copy_rebases_cleanly() {
    let home = seeded(1);
    let bare = bare_remote();
    track_origin(home.path(), bare.path());
    let name = only_memory(home.path());
    // "Another machine" pushes the very same file first.
    let other = clone_of(bare.path());
    std::fs::create_dir_all(other.path().join("memories")).expect("memories dir");
    std::fs::copy(
        home.path().join("memories").join(&name),
        other.path().join("memories").join(&name),
    )
    .expect("copy the memory");
    commit_all(other.path(), "same memory from elsewhere");
    run_git(other.path(), &["push", "-q"]);
    let upstream = remote_head(&bare);

    let (done, log) = run_sync(home.path(), &Config::defaults(), Some(true));
    let done = done.expect("an identical upstream copy must not fail the sync");

    assert!(
        done.committed,
        "the untracked memory was committed: {done:?}"
    );
    assert!(done.pulled && done.pushed, "{done:?}");
    assert_eq!(
        done.commit.as_deref(),
        Some(upstream.as_str()),
        "the local commit was already upstream, so HEAD after the rebase IS the upstream commit"
    );
    assert_eq!(remote_head(&bare), upstream);
    assert!(
        git_stdout(
            home.path(),
            &["ls-tree", "--name-only", "HEAD", "memories/"]
        )
        .contains(&name),
        "the memory is tracked in HEAD"
    );
    let commit_at = log
        .iter()
        .position(|l| l.starts_with("git commit"))
        .expect("commit logged");
    let pull_at = log
        .iter()
        .position(|l| l.starts_with("git pull"))
        .expect("pull logged");
    assert!(commit_at < pull_at, "commit must precede pull: {log:?}");
}

/// The other half of committing first: a DIFFERENT upstream file at the same
/// path is now a real rebase `CONFLICT` — aborted, reported by path, nothing
/// pushed, the local sync commit intact — instead of the generic pull error.
#[test]
fn sync_reports_a_same_path_conflict_by_path_and_aborts_the_rebase() {
    let home = seeded(1);
    let bare = bare_remote();
    track_origin(home.path(), bare.path());
    let name = only_memory(home.path());
    let other = clone_of(bare.path());
    std::fs::create_dir_all(other.path().join("memories")).expect("memories dir");
    std::fs::write(
        other.path().join("memories").join(&name),
        "a different body at the same path\n",
    )
    .expect("write the conflicting memory");
    commit_all(other.path(), "conflicting memory from elsewhere");
    run_git(other.path(), &["push", "-q"]);
    let upstream = remote_head(&bare);
    let local_body =
        std::fs::read_to_string(home.path().join("memories").join(&name)).expect("local body");

    let (done, _log) = run_sync(home.path(), &Config::defaults(), Some(true));
    let err = done.expect_err("an add/add conflict must fail the sync");

    let msg = err.to_string();
    assert!(
        msg.contains(&format!("sync conflict in: memories/{name}")),
        "the conflict names its path: {msg}"
    );
    assert!(
        !msg.contains("git pull failed"),
        "never the generic error: {msg}"
    );
    assert_eq!(remote_head(&bare), upstream, "nothing may be pushed");
    assert!(
        !home.path().join(".git/rebase-merge").exists(),
        "the stopped rebase must be aborted"
    );
    assert_eq!(
        git_stdout(home.path(), &["show", &format!("HEAD:memories/{name}")]),
        local_body,
        "the local sync commit survives the abort"
    );
    assert!(
        git_stdout(home.path(), &["status", "--porcelain", "--", "memories"])
            .trim()
            .is_empty(),
        "the work tree is left as it was found"
    );
}

/// `git commit` is pathspec-limited to `memories/`: a store living inside a
/// larger repo (a dotfiles checkout) must not sweep whatever else the
/// operator had staged into the sync commit.
#[test]
fn sync_commit_is_pathspec_limited_and_leaves_unrelated_staged_files_staged() {
    let home = seeded(1);
    init_repo(home.path());
    run_git(
        home.path(),
        &["commit", "-q", "--allow-empty", "-m", "init"],
    );
    std::fs::write(home.path().join("dotfile.txt"), "unrelated staged change\n")
        .expect("write dotfile");
    run_git(home.path(), &["add", "dotfile.txt"]);
    let name = only_memory(home.path());

    let (done, _log) = run_sync(home.path(), &Config::defaults(), Some(false));
    let done = done.expect("sync");

    assert!(done.committed, "{done:?}");
    assert!(!done.pulled && !done.pushed, "no upstream: {done:?}");
    let in_commit = git_stdout(
        home.path(),
        &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"],
    );
    assert_eq!(
        in_commit.trim(),
        format!("memories/{name}"),
        "only memories/ may be in the sync commit"
    );
    let still_staged = git_stdout(home.path(), &["diff", "--cached", "--name-only"]);
    assert_eq!(
        still_staged.trim(),
        "dotfile.txt",
        "the operator's staged file is left staged, untouched"
    );
}

/// `[git] remote` is honored: the push goes to that remote as `git push
/// <remote> HEAD`, and the branch needs no upstream for it to run.
#[test]
fn sync_pushes_to_the_configured_git_remote_even_without_an_upstream() {
    let home = seeded(1);
    init_repo(home.path());
    run_git(
        home.path(),
        &["commit", "-q", "--allow-empty", "-m", "init"],
    );
    let backup = bare_remote();
    run_git(
        home.path(),
        &["remote", "add", "backup", &backup.path().to_string_lossy()],
    );
    let mut cfg = Config::defaults();
    cfg.git.remote = "backup".to_string();

    let (done, log) = run_sync(home.path(), &cfg, Some(true));
    let done = done.expect("sync");

    assert!(!done.pulled, "no upstream, so no pull: {done:?}");
    assert!(
        done.pushed,
        "a configured remote needs no upstream: {done:?}"
    );
    let commit = done.commit.expect("commit oid");
    assert_eq!(
        remote_head(&backup),
        commit,
        "the configured remote advanced"
    );
    assert!(
        log.iter().any(|l| l == "git push backup HEAD"),
        "the push names the configured remote: {log:?}"
    );
}

/// An autostash whose re-apply conflicts exits ZERO from `git pull`; the
/// unmerged-path check after the pull is what catches it. Because the
/// memories were committed first, the conflict can only be the operator's
/// own edit to a tracked non-memory file: it is reported by path, left as
/// git left it (stash kept), and nothing is pushed.
#[test]
fn sync_reports_an_autostash_conflict_by_path_and_pushes_nothing() {
    let home = seeded(1);
    let bare = bare_remote();
    init_repo(home.path());
    std::fs::write(home.path().join("notes.txt"), "one\n").expect("write notes");
    run_git(home.path(), &["add", "notes.txt"]);
    run_git(
        home.path(),
        &["-c", "commit.gpgsign=false", "commit", "-q", "-m", "init"],
    );
    run_git(
        home.path(),
        &["remote", "add", "origin", &bare.path().to_string_lossy()],
    );
    run_git(home.path(), &["push", "-q", "-u", "origin", "main"]);
    let other = clone_of(bare.path());
    std::fs::write(other.path().join("notes.txt"), "upstream\n").expect("write notes");
    commit_all(other.path(), "notes from elsewhere");
    run_git(other.path(), &["push", "-q"]);
    let upstream = remote_head(&bare);
    // The operator's own uncommitted edit to a tracked, non-memory file.
    std::fs::write(home.path().join("notes.txt"), "local\n").expect("edit notes");

    let (done, _log) = run_sync(home.path(), &Config::defaults(), Some(true));
    let err = done.expect_err("a conflicting autostash re-apply must fail the sync");

    let msg = err.to_string();
    assert!(
        msg.contains("sync conflict in: notes.txt"),
        "the conflict names the operator's file: {msg}"
    );
    assert_eq!(remote_head(&bare), upstream, "nothing may be pushed");
    assert!(
        git_stdout(home.path(), &["stash", "list"]).contains("autostash"),
        "the operator's edit stays in the stash git created; the job must not drop it"
    );
    assert_eq!(
        git_stdout(home.path(), &["diff", "--name-only", "--diff-filter=U"]).trim(),
        "notes.txt",
        "left exactly as git left it"
    );
    assert_eq!(
        git_stdout(home.path(), &["rev-list", "--count", "origin/main..HEAD"]).trim(),
        "1",
        "the memory commit itself was rebased onto upstream; only the push was withheld"
    );
}
