//! `api::setup::plan` — the pure planner. Every snapshot here is built by
//! running the real `detect` against a real temp data dir and a real git
//! working tree, so the inputs are the ones production produces.
use super::run;
use crate::api::setup::detect::{self, Detected};
use crate::api::setup::{
    AGENT_HOST, CLOUD_AUTH, DATA_DIR, GIT_HOOKS, INDEX_CODE, INDEX_DOCS, REINFORCE, Request, Step,
    StepState,
};
use crate::config::{Config, Paths};
use crate::test_common::{git_commit::commit_files, git_repo::init_repo};
use crate::utilities::context::Ctx;

/// Detect against a real data dir and a real target path.
fn detect_at(data: &std::path::Path, target: &std::path::Path) -> Detected {
    let paths = Paths::new(data.to_path_buf());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    detect::run(&mut ctx, target, Some("definitely-not-a-host")).unwrap()
}

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

/// The step with `id`, which `run` always emits.
fn step<'a>(steps: &'a [Step], id: &str) -> &'a Step {
    steps
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("plan must always contain `{id}`"))
}

#[test]
fn a_fresh_repo_plans_hooks_and_indexing_as_pending() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();
    let detected = detect_at(data.path(), repo.path());

    let steps = run(&detected, &Request::default());

    assert_eq!(
        steps.iter().map(|s| s.id).collect::<Vec<_>>(),
        crate::api::setup::STEP_IDS,
        "every id appears exactly once, in order"
    );
    assert_eq!(step(&steps, DATA_DIR).state, StepState::Pending);
    assert_eq!(step(&steps, GIT_HOOKS).state, StepState::Pending);
    assert_eq!(step(&steps, INDEX_CODE).state, StepState::Pending);
    assert_eq!(
        step(&steps, REINFORCE).state,
        StepState::Satisfied,
        "`reinforce.enabled` ships as true, so a fresh machine has nothing \
         to do here — it is pending only for someone who turned it off"
    );
    assert_eq!(
        step(&steps, INDEX_CODE).detail,
        "never indexed",
        "a repo with no repos row has never been indexed"
    );
}

#[test]
fn reinforce_is_pending_only_once_it_has_been_turned_off() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();
    let mut detected = detect_at(data.path(), repo.path());
    detected.reinforce_enabled = false;

    let steps = run(&detected, &Request::default());

    assert_eq!(step(&steps, REINFORCE).state, StepState::Pending);
    assert!(
        step(&steps, REINFORCE).detail.contains("search"),
        "the detail explains what it does: {}",
        step(&steps, REINFORCE).detail
    );
}

#[test]
fn a_non_git_target_makes_every_repo_scoped_step_unavailable() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let detected = detect_at(data.path(), work.path());

    let steps = run(&detected, &Request::default());

    for id in [GIT_HOOKS, INDEX_CODE, INDEX_DOCS] {
        match &step(&steps, id).state {
            StepState::Unavailable { reason } => {
                assert_eq!(reason, "not a git repository", "wrong reason for {id}");
            }
            other => panic!("{id} should be unavailable, got {other:?}"),
        }
    }
    // A machine-level step is unaffected by the target not being a repo.
    assert_eq!(step(&steps, DATA_DIR).state, StepState::Pending);
}

#[test]
fn cloud_auth_is_always_a_report_never_something_setup_can_fail_at() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();
    let detected = detect_at(data.path(), repo.path());

    // There is no mode in which this becomes Pending: setup points at
    // `comemory auth login` rather than reimplementing the device flow, so
    // it can never be selected and can never fail.
    for request in [
        Request::default(),
        Request {
            only: vec![CLOUD_AUTH.to_string()],
            ..Request::default()
        },
    ] {
        let steps = run(&detected, &request);
        match &step(&steps, CLOUD_AUTH).state {
            StepState::Unavailable { reason } => {
                assert!(
                    reason.contains("comemory auth login"),
                    "the reason must name the remedy, got {reason}"
                );
            }
            other => panic!("cloud-auth must never be actionable, got {other:?}"),
        }
    }
}

#[test]
fn a_foreign_hook_is_unavailable_with_the_force_remedy_not_pending() {
    let data = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    init_repo(repo.path());
    let hooks = repo.path().join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(hooks.join("post-commit"), "#!/bin/sh\necho mine\n").unwrap();
    let detected = detect_at(data.path(), repo.path());

    let steps = run(&detected, &Request::default());

    match &step(&steps, GIT_HOOKS).state {
        StepState::Unavailable { reason } => {
            assert!(reason.contains("post-commit"), "names the hook: {reason}");
            assert!(reason.contains("--force"), "names the remedy: {reason}");
        }
        other => panic!("a foreign hook must not be pending, got {other:?}"),
    }
}

#[test]
fn only_and_skip_deselect_pending_steps_and_skip_wins() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();
    let detected = detect_at(data.path(), repo.path());

    let only = run(
        &detected,
        &Request {
            only: vec![GIT_HOOKS.to_string()],
            ..Request::default()
        },
    );
    assert_eq!(step(&only, GIT_HOOKS).state, StepState::Pending);
    assert_eq!(step(&only, INDEX_CODE).state, StepState::Skipped);
    assert_eq!(step(&only, DATA_DIR).state, StepState::Skipped);

    let both = run(
        &detected,
        &Request {
            only: vec![GIT_HOOKS.to_string()],
            skip: vec![GIT_HOOKS.to_string()],
            ..Request::default()
        },
    );
    assert_eq!(
        step(&both, GIT_HOOKS).state,
        StepState::Skipped,
        "--skip wins over --only: deselection is the safe direction"
    );
}

#[test]
fn deselection_never_rewrites_an_unavailable_step() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let detected = detect_at(data.path(), work.path());

    let steps = run(
        &detected,
        &Request {
            only: vec![DATA_DIR.to_string()],
            ..Request::default()
        },
    );

    // `--only data-dir` deselects the other *pending* steps, but a step that
    // cannot run here stays unavailable — the operator still needs the reason.
    assert!(matches!(
        step(&steps, GIT_HOOKS).state,
        StepState::Unavailable { .. }
    ));
}

#[test]
fn no_agent_host_on_path_is_unavailable_and_names_the_hosts() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let detected = detect_at(data.path(), work.path());

    let steps = run(&detected, &Request::default());

    match &step(&steps, AGENT_HOST).state {
        StepState::Unavailable { reason } => {
            assert!(
                reason.contains("PATH"),
                "explains where we looked: {reason}"
            );
        }
        other => panic!("no host present means unavailable, got {other:?}"),
    }
}

#[test]
fn nothing_is_pending_when_every_step_is_satisfied_or_unavailable() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut detected = detect_at(data.path(), work.path());
    // Everything machine-level already done; the repo-scoped steps are
    // unavailable because the target is not a git tree.
    detected.db_writable = true;
    detected.reinforce_enabled = true;
    detected.authenticated = true;

    let steps = run(&detected, &Request::default());

    assert!(
        !steps.iter().any(|s| s.state.is_pending()),
        "the wizard must be able to short-circuit instead of \
         building an empty multiselect: {steps:?}"
    );
}
