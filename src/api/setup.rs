//! `api::setup::{Request, Response, run}` — the shared middle of
//! `comemory setup`, which takes a machine or a repo from "binary
//! installed" to "memory + code search working in my agent".
//!
//! Like [`crate::api::overview`], a composition and nothing else: it
//! delegates to the existing readers and writers rather than restating
//! them. [`detect`] reads (never creating the database — asking what needs
//! setting up is a read), [`plan`] is pure, [`apply`] is the only phase
//! that writes. Every decision lives in [`plan`]; the wizard only selects,
//! which is what makes this testable without a pty.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::prelude::*;

/// The write phase: dispatch each pending step to its existing writer.
pub mod apply;
/// The read-only environment probe.
pub mod detect;
/// The pure snapshot-to-step-list planner.
pub mod plan;

/// Stable step ids, in the order a plan lists them. They are a public
/// contract: `--only` / `--skip` name them and the JSON envelope carries
/// them.
pub const STEP_IDS: &[&str] = &[
    DATA_DIR, AGENT_HOST, GIT_HOOKS, INDEX_CODE, INDEX_DOCS, REINFORCE, CLOUD_AUTH,
];

/// The data directory and its SQLite mirror.
pub const DATA_DIR: &str = "data-dir";
/// Bundled skills and hooks inside an agent host.
pub const AGENT_HOST: &str = "agent-host";
/// The three git reindex hooks in this repo.
pub const GIT_HOOKS: &str = "git-hooks";
/// This repo's code index.
pub const INDEX_CODE: &str = "index-code";
/// Registered document sources for this repo.
pub const INDEX_DOCS: &str = "index-docs";
/// Search-edit auto-reinforcement (`[reinforce]` in `config.toml`).
pub const REINFORCE: &str = "reinforce";
/// Cloud workspace sign-in.
pub const CLOUD_AUTH: &str = "cloud-auth";

/// `comemory setup` request.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Repo root for the repo-scoped steps. Defaults to the working
    /// directory.
    #[serde(default)]
    pub repo: Option<String>,
    /// Restrict the `agent-host` step to one host (`claude` / `codex`).
    /// `None` means every host whose CLI answers `--version`.
    #[serde(default)]
    pub host: Option<String>,
    /// Step ids to run; every other step is [`StepState::Skipped`].
    #[serde(default)]
    pub only: Vec<String>,
    /// Step ids to force to [`StepState::Skipped`].
    #[serde(default)]
    pub skip: Vec<String>,
    /// Whether to apply the plan. `false` is the `--dry-run` shape.
    #[serde(default)]
    pub apply: bool,
}

/// What a step's plan resolved to. `plan` emits only `Satisfied`, `Pending`,
/// `Skipped`, and `Unavailable`; `apply` moves `Pending` to `Applied` or
/// `Failed` and leaves the other three untouched.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum StepState {
    /// Already done; apply skips it.
    Satisfied,
    /// Selected and not yet done; apply will run it.
    Pending,
    /// Deselected by `--only` / `--skip` or by the wizard.
    Skipped,
    /// Cannot run here. Never a failure — the machine simply cannot offer it.
    Unavailable {
        /// Why, in terms the operator can act on.
        reason: String,
    },
    /// Apply ran it successfully.
    Applied {
        /// One line describing what changed.
        detail: String,
    },
    /// Apply ran it and it returned an error.
    Failed {
        /// The error's `Display`.
        error: String,
    },
}

impl StepState {
    /// Whether [`apply`] should run this step.
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }
}

/// One planned step.
#[derive(Serialize, Debug, Clone)]
pub struct Step {
    /// One of [`STEP_IDS`].
    pub id: &'static str,
    /// Short human title, shown in the wizard and the summary.
    pub title: String,
    /// One line of detected context (schema version, index freshness, …).
    pub detail: String,
    /// The resolved state.
    #[serde(flatten)]
    pub state: StepState,
}

/// The repo the repo-scoped steps act on.
#[derive(Serialize, Debug, Clone)]
pub struct RepoContext {
    /// The repo label memories and symbols are filed under.
    pub label: String,
    /// The working-tree root.
    pub root: String,
    /// Whether that root is inside a git working tree.
    pub is_git: bool,
}

/// `comemory setup` response: the whole plan, plus per-state counts so a
/// caller can branch without walking the list.
#[derive(Serialize, Debug)]
pub struct Response {
    /// The resolved data directory.
    pub data_dir: String,
    /// The repo context, when the target path is inside a git working tree.
    pub repo: Option<RepoContext>,
    /// Whether this run applied nothing by request.
    pub dry_run: bool,
    /// Every step, in [`STEP_IDS`] order.
    pub steps: Vec<Step>,
    /// Steps that were already done.
    pub satisfied: usize,
    /// Steps this run applied.
    pub applied: usize,
    /// Steps this run tried and failed.
    pub failed: usize,
    /// Steps deselected by the caller.
    pub skipped: usize,
}

impl Response {
    /// Build a response from a finished step list, counting the states.
    fn from_steps(
        data_dir: String,
        repo: Option<RepoContext>,
        dry_run: bool,
        steps: Vec<Step>,
    ) -> Self {
        let count =
            |matcher: fn(&StepState) -> bool| steps.iter().filter(|s| matcher(&s.state)).count();
        Self {
            data_dir,
            repo,
            dry_run,
            satisfied: count(|s| matches!(s, StepState::Satisfied)),
            applied: count(|s| matches!(s, StepState::Applied { .. })),
            failed: count(|s| matches!(s, StepState::Failed { .. })),
            skipped: count(|s| matches!(s, StepState::Skipped)),
            steps,
        }
    }

    /// The ids of every step that failed, for the caller's error message.
    pub fn failed_ids(&self) -> Vec<&'static str> {
        self.steps
            .iter()
            .filter(|s| matches!(s.state, StepState::Failed { .. }))
            .map(|s| s.id)
            .collect()
    }
}

/// Validate `ids` against [`STEP_IDS`], naming `flag` and the offending id.
///
/// # Errors
/// [`Error::Usage`] on the first unknown id, listing the known ones.
pub fn validate_step_ids(ids: &[String], flag: &str) -> Result<()> {
    for id in ids {
        if !STEP_IDS.contains(&id.as_str()) {
            return Err(Error::Usage(format!(
                "{flag}: unknown step `{id}` (known: {})",
                STEP_IDS.join(", ")
            )));
        }
    }
    Ok(())
}

/// Detect, plan, and — when `req.apply` — apply.
///
/// Step ids are validated before anything is probed, so a typo costs nothing
/// and cannot leave a half-applied plan behind.
///
/// # Errors
/// [`Error::Usage`] for an unknown step id or agent host. A step that fails
/// while being applied is recorded as [`StepState::Failed`] and does not
/// abort the run — the caller decides what a failure means for its exit code.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    validate_step_ids(&req.only, "--only")?;
    validate_step_ids(&req.skip, "--skip")?;
    if let Some(host) = &req.host {
        crate::api::install::validate_host(host)?;
    }
    let data_dir = ctx.paths.data_dir().display().to_string();
    let target = req
        .repo
        .clone()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let detected = detect::run(ctx, &target, req.host.as_deref())?;
    let repo = detected.repo.clone();
    let mut steps = plan::run(&detected, &req);
    if req.apply {
        apply::run(ctx, &detected, &mut steps);
    }
    Ok(Response::from_steps(data_dir, repo, !req.apply, steps))
}

#[cfg(test)]
#[path = "tests/setup.rs"]
mod tests;
