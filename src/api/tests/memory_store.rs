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

use comemory::api::{self, Ctx, memory_store};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::store::connection;
use tempfile::TempDir;

use crate::test_common::git_repo::{init_repo, run_git};

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
    let bare = TempDir::new().expect("bare tempdir");
    std::fs::create_dir_all(bare.path()).expect("bare dir");
    run_git(
        bare.path(),
        &["init", "-q", "--bare", "--initial-branch=main"],
    );
    init_repo(home.path());
    run_git(
        home.path(),
        &["commit", "-q", "--allow-empty", "-m", "init"],
    );
    run_git(
        home.path(),
        &["remote", "add", "origin", &bare.path().to_string_lossy()],
    );
    run_git(home.path(), &["push", "-q", "-u", "origin", "main"]);
    let before = remote_head(&bare);

    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let sink = std::cell::RefCell::new(Vec::<String>::new());
    let done = memory_store::sync(
        &mut ctx,
        memory_store::STORE_ID,
        &memory_store::SyncRequest { push: Some(true) },
        |line| sink.borrow_mut().push(line.to_string()),
    )
    .expect("sync");
    let log = sink.into_inner();

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
