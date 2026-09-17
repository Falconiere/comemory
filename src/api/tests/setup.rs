//! `api::setup::run` — the composition: validation before probing, and the
//! counts the response carries.
use super::{Request, Response, STEP_IDS, run, validate_step_ids};
use crate::api::setup::StepState;
use crate::config::{Config, Paths};
use crate::test_common::{git_commit::commit_files, git_repo::init_repo};
use crate::utilities::context::Ctx;

/// A real git working tree with two real Rust files.
fn repo_with_sources() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    init_repo(repo.path());
    commit_files(
        repo.path(),
        &[
            ("src/alpha.rs", "pub fn alpha() -> u8 { 1 }\n"),
            ("src/beta.rs", "pub fn beta(n: u8) -> u8 { n + 1 }\n"),
        ],
        "seed",
    );
    repo
}

/// Run `api::setup::run` against a real data dir and a real target.
fn run_at(data: &std::path::Path, req: Request) -> crate::prelude::Result<Response> {
    let paths = Paths::new(data.to_path_buf());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    run(&mut ctx, req)
}

#[test]
fn an_unknown_step_id_is_rejected_before_anything_is_probed() {
    let data = tempfile::tempdir().unwrap();
    let err = run_at(
        data.path(),
        Request {
            only: vec!["bogus".to_string()],
            ..Request::default()
        },
    )
    .unwrap_err();

    let message = err.to_string();
    assert!(message.contains("bogus"), "names the bad id: {message}");
    assert!(message.contains("--only"), "names the flag: {message}");
    assert!(message.contains("git-hooks"), "lists known ids: {message}");
    assert!(
        !data.path().join("comemory.db").exists(),
        "validation must precede detection, so nothing is created"
    );
}

#[test]
fn an_unknown_host_is_rejected_the_same_way() {
    let data = tempfile::tempdir().unwrap();
    let err = run_at(
        data.path(),
        Request {
            host: Some("emacs".to_string()),
            ..Request::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("emacs"));
}

#[test]
fn validate_step_ids_accepts_every_published_id() {
    let all: Vec<String> = STEP_IDS.iter().map(|id| (*id).to_string()).collect();
    validate_step_ids(&all, "--only").unwrap();
}

#[test]
fn a_dry_run_reports_every_step_and_applies_nothing() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();

    let resp = run_at(
        data.path(),
        Request {
            repo: Some(repo.path().display().to_string()),
            host: Some("codex".to_string()),
            apply: false,
            ..Request::default()
        },
    )
    .unwrap();

    assert!(resp.dry_run);
    assert_eq!(resp.steps.len(), STEP_IDS.len());
    assert_eq!(resp.applied, 0);
    assert_eq!(resp.failed, 0);
    assert!(
        !data.path().join("comemory.db").exists(),
        "a dry run creates no database"
    );
    assert!(resp.repo.is_some(), "the target is a real git worktree");
    assert!(resp.failed_ids().is_empty());
    // Counts agree with the list they summarize.
    let satisfied = resp
        .steps
        .iter()
        .filter(|s| matches!(s.state, StepState::Satisfied))
        .count();
    assert_eq!(resp.satisfied, satisfied);
}

#[test]
fn applying_git_hooks_reports_one_applied_step_and_no_failures() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();

    let resp = run_at(
        data.path(),
        Request {
            repo: Some(repo.path().display().to_string()),
            host: Some("codex".to_string()),
            only: vec![crate::api::setup::GIT_HOOKS.to_string()],
            apply: true,
            ..Request::default()
        },
    )
    .unwrap();

    assert!(!resp.dry_run);
    assert_eq!(resp.applied, 1, "exactly the selected step ran");
    assert_eq!(resp.failed, 0);
    assert!(resp.failed_ids().is_empty());
    assert!(
        resp.skipped >= 1,
        "the unselected pending steps are reported skipped"
    );
}
