//! `domains::integrations::setup::apply` — the write phase, against a real temporary data
//! directory and a real `git init` working tree carrying real Rust files.
use super::run;
use crate::config::{Config, Paths};
use crate::domains::code::git_utils;
use crate::domains::integrations::setup::detect::{self, Detected};
use crate::domains::integrations::setup::{GIT_HOOKS, INDEX_CODE, Request, Step, StepState, plan};
use crate::test_common::{git_commit::commit_files, git_repo::init_repo};
use crate::utilities::context::Ctx;

/// Four real Rust files, enough to prove a walk without indexing a whole
/// crate on every test run.
const SOURCES: &[(&str, &str)] = &[
    ("src/alpha.rs", "pub fn alpha() -> u8 { 1 }\n"),
    ("src/beta.rs", "pub fn beta(n: u8) -> u8 { n + 1 }\n"),
    (
        "src/gamma.rs",
        "pub struct Gamma { pub n: u8 }\nimpl Gamma { pub fn new() -> Self { Self { n: 0 } } }\n",
    ),
    ("src/delta.rs", "pub const DELTA: &str = \"delta\";\n"),
];

/// A real git working tree with [`SOURCES`] committed.
fn repo_with_sources() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    init_repo(repo.path());
    commit_files(repo.path(), SOURCES, "seed");
    repo
}

/// Detect, plan, and apply against `data` / `repo`, returning the steps.
fn detect_plan_apply(
    data: &std::path::Path,
    repo: &std::path::Path,
    req: &Request,
) -> (Detected, Vec<Step>) {
    let paths = Paths::new(data.to_path_buf());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let detected = detect::run(&mut ctx, repo, Some("definitely-not-a-host")).unwrap();
    let mut steps = plan::run(&detected, req);
    run(&mut ctx, &detected, &mut steps);
    (detected, steps)
}

/// The step with `id`.
fn step<'a>(steps: &'a [Step], id: &str) -> &'a Step {
    steps
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("plan must contain `{id}`"))
}

#[test]
fn git_hooks_writes_every_hook_and_a_second_run_reports_satisfied() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();
    let req = Request {
        only: vec![GIT_HOOKS.to_string()],
        apply: true,
        ..Request::default()
    };

    let (_, steps) = detect_plan_apply(data.path(), repo.path(), &req);

    assert!(
        matches!(step(&steps, GIT_HOOKS).state, StepState::Applied { .. }),
        "got {:?}",
        step(&steps, GIT_HOOKS).state
    );
    for hook in crate::domains::code::hooks::GIT_HOOKS {
        assert!(
            git_utils::hook_installed(repo.path(), hook),
            "{hook} must be installed and carry comemory's marker"
        );
    }

    // Idempotent: re-running finds nothing left to do.
    let (_, again) = detect_plan_apply(data.path(), repo.path(), &req);
    assert_eq!(step(&again, GIT_HOOKS).state, StepState::Satisfied);
}

#[test]
fn index_code_indexes_the_real_sources_and_then_reports_fresh() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();
    let req = Request {
        only: vec![INDEX_CODE.to_string()],
        apply: true,
        ..Request::default()
    };

    let (_, steps) = detect_plan_apply(data.path(), repo.path(), &req);

    match &step(&steps, INDEX_CODE).state {
        StepState::Applied { detail } => {
            assert!(
                detail.contains("file(s)"),
                "the applied detail reports the real walk, got {detail}"
            );
        }
        other => panic!("index-code should have applied, got {other:?}"),
    }

    // The freshness verdict comes from `domains::code::repos` reading real HEAD oids.
    let (detected, again) = detect_plan_apply(data.path(), repo.path(), &req);
    assert_eq!(detected.index_status.as_deref(), Some("fresh"));
    assert_eq!(step(&again, INDEX_CODE).state, StepState::Satisfied);
}

#[test]
fn a_failing_step_is_recorded_and_the_other_steps_still_run() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();
    let paths = Paths::new(data.path().to_path_buf());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let detected = detect::run(&mut ctx, repo.path(), Some("definitely-not-a-host")).unwrap();

    // `cloud-auth` is a report with no applier at all, so forcing it Pending
    // by hand is the honest way to make exactly one step fail while its
    // neighbours still run — and it also pins the refusal that guards any
    // step a caller forces Pending.
    let mut steps = plan::run(
        &detected,
        &Request {
            apply: true,
            ..Request::default()
        },
    );
    for candidate in &mut steps {
        if candidate.id == crate::domains::integrations::setup::CLOUD_AUTH {
            candidate.state = StepState::Pending;
        }
    }
    run(&mut ctx, &detected, &mut steps);

    assert!(
        matches!(
            step(&steps, crate::domains::integrations::setup::CLOUD_AUTH).state,
            StepState::Failed { .. }
        ),
        "the forced step must fail"
    );
    assert!(
        matches!(step(&steps, GIT_HOOKS).state, StepState::Applied { .. }),
        "a neighbouring step still runs after a failure: {:?}",
        step(&steps, GIT_HOOKS).state
    );
    for hook in crate::domains::code::hooks::GIT_HOOKS {
        assert!(git_utils::hook_installed(repo.path(), hook));
    }
}

#[test]
fn apply_leaves_a_non_pending_step_exactly_as_planned() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::new(data.path().to_path_buf());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let detected = detect::run(&mut ctx, work.path(), Some("definitely-not-a-host")).unwrap();
    let mut steps = plan::run(
        &detected,
        &Request {
            apply: true,
            only: vec!["data-dir".to_string()],
            ..Request::default()
        },
    );

    run(&mut ctx, &detected, &mut steps);

    // Unavailable steps are untouched by apply — no reason is lost.
    match &step(&steps, GIT_HOOKS).state {
        StepState::Unavailable { reason } => assert_eq!(reason, "not a git repository"),
        other => panic!("expected the planned unavailable state, got {other:?}"),
    }
}
