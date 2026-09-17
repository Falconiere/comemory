#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/code/install_hooks.rs`. Calls `comemory::domains::code::install_hooks::run`
//! directly against a `Ctx::lazy` (conn-free) — proving the three hooks are
//! written, the pre-flight refuses to clobber a FOREIGN hook without `force`,
//! that an outdated comemory-written hook is repaired without it, and that
//! `force` overwrites (`cli::install_hooks::run` is byte-compat
//! tested against CLI stdout in `tests/cli__install_hooks.rs`; the HTTP
//! route, including `--repo` containment, lives in
//! `tests/serve__routes__maint__admin.rs`).

use comemory::config::{Config, Paths};
use comemory::utilities::context::Ctx;

fn ctx_paths(home: &std::path::Path) -> Paths {
    Paths::new(home)
}

fn request(repo: &std::path::Path, force: bool) -> comemory::domains::code::install_hooks::Request {
    comemory::domains::code::install_hooks::Request {
        repo: repo.display().to_string(),
        force,
    }
}

#[test]
fn run_installs_all_three_hooks() {
    let home = tempfile::tempdir().expect("tempdir");
    let repo = home.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("fake .git dir");

    let paths = ctx_paths(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = comemory::domains::code::install_hooks::run(&mut ctx, request(&repo, false))
        .expect("install run");

    assert_eq!(resp.installed.len(), 3);
    assert_eq!(resp.repo, repo.display().to_string());
    for hook in ["post-commit", "post-merge", "post-checkout"] {
        assert!(
            repo.join(".git").join("hooks").join(hook).exists(),
            "{hook} must be written"
        );
    }
}

#[test]
fn run_refuses_to_clobber_a_foreign_hook_without_force() {
    let home = tempfile::tempdir().expect("tempdir");
    let repo = home.path().join("repo");
    let hooks_dir = repo.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("fake hooks dir");
    std::fs::write(
        hooks_dir.join("post-commit"),
        "#!/bin/sh\necho hand-written\n",
    )
    .expect("write pre-existing hook");

    let paths = ctx_paths(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let err = comemory::domains::code::install_hooks::run(&mut ctx, request(&repo, false))
        .expect_err("must refuse");
    assert!(err.to_string().contains("already exists"));

    let body = std::fs::read_to_string(hooks_dir.join("post-commit")).expect("read hook");
    assert!(
        body.contains("hand-written"),
        "pre-existing hook must survive a refused install"
    );
    assert!(
        !hooks_dir.join("post-merge").exists(),
        "no partial install: post-merge must not be written either"
    );
}

#[test]
fn run_force_overwrites_an_existing_hook() {
    let home = tempfile::tempdir().expect("tempdir");
    let repo = home.path().join("repo");
    let hooks_dir = repo.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("fake hooks dir");
    std::fs::write(
        hooks_dir.join("post-commit"),
        "#!/bin/sh\necho hand-written\n",
    )
    .expect("write pre-existing hook");

    let paths = ctx_paths(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = comemory::domains::code::install_hooks::run(&mut ctx, request(&repo, true))
        .expect("force install");
    assert_eq!(resp.installed.len(), 3);

    let body = std::fs::read_to_string(hooks_dir.join("post-commit")).expect("read hook");
    assert!(
        body.contains("index-code"),
        "force must overwrite the hand-written hook"
    );
}

#[test]
fn request_defaults_repo_to_dot() {
    let req: comemory::domains::code::install_hooks::Request =
        serde_json::from_value(serde_json::json!({})).expect("defaults must parse");
    assert_eq!(req.repo, ".");
    assert!(!req.force);
}

#[test]
fn request_rejects_unknown_fields() {
    let err = serde_json::from_value::<comemory::domains::code::install_hooks::Request>(
        serde_json::json!({
            "repo": ".",
            "bogus": 1,
        }),
    )
    .expect_err("unknown field must be rejected");
    assert!(err.to_string().contains("unknown field"));
}

/// The body comemory shipped before the worktree-label rule: it carries the
/// `comemory index-code` marker, so `hook_installed` has always reported it as
/// installed and no release ever replaced it — while it kept passing the
/// checkout's OWN basename as `--repo`, minting one repo per `git worktree
/// add`. `install-hooks` must repair its own hook without `--force`.
const LEGACY_HOOK_SCRIPT: &str = "#!/usr/bin/env bash\n\
     ROOT=\"$(git rev-parse --show-toplevel 2>/dev/null)\"\n\
     [ -z \"$ROOT\" ] && exit 0\n\
     REPO=\"$(basename \"$ROOT\")\"\n\
     ( comemory index-code --repo \"$REPO\" --path \"$ROOT\" >/dev/null 2>&1 & )\n\
     exit 0\n";

#[test]
fn run_repairs_an_outdated_comemory_hook_without_force() {
    let home = tempfile::tempdir().expect("tempdir");
    let repo = home.path().join("repo");
    let hooks_dir = repo.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("fake hooks dir");
    for hook in ["post-commit", "post-merge", "post-checkout"] {
        std::fs::write(hooks_dir.join(hook), LEGACY_HOOK_SCRIPT).expect("write legacy hook");
    }

    let paths = ctx_paths(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = comemory::domains::code::install_hooks::run(&mut ctx, request(&repo, false))
        .expect("repair without force");
    assert_eq!(resp.installed.len(), 3);

    for hook in ["post-commit", "post-merge", "post-checkout"] {
        let body = std::fs::read_to_string(hooks_dir.join(hook)).expect("read hook");
        assert_eq!(
            body,
            comemory::git_utils::REINDEX_HOOK_SCRIPT,
            "{hook} must be byte-identical to the body this binary ships"
        );
        assert!(
            !comemory::git_utils::hook_outdated(&repo, hook),
            "{hook} must no longer read as outdated after the repair"
        );
    }
}

#[test]
fn run_is_idempotent_over_an_up_to_date_comemory_hook() {
    let home = tempfile::tempdir().expect("tempdir");
    let repo = home.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("fake .git dir");

    let paths = ctx_paths(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    comemory::domains::code::install_hooks::run(&mut ctx, request(&repo, false))
        .expect("first install");
    let first = std::fs::read_to_string(repo.join(".git").join("hooks").join("post-commit"))
        .expect("read hook");

    comemory::domains::code::install_hooks::run(&mut ctx, request(&repo, false))
        .expect("re-running over our own current hook is not a conflict");
    let second = std::fs::read_to_string(repo.join(".git").join("hooks").join("post-commit"))
        .expect("read hook again");
    assert_eq!(first, second);
}
