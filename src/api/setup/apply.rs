//! The write half of `comemory setup`: run each pending step through the
//! command that already owns it.
//!
//! **A failing step never aborts the run.** The steps are independent
//! enablements, so one that fails is recorded as [`StepState::Failed`] and
//! the rest still run; the caller turns any failure into its exit code after
//! rendering the whole summary. Only a failure to *iterate* — which cannot
//! happen here — would propagate.

use super::detect::Detected;
use super::{AGENT_HOST, DATA_DIR, GIT_HOOKS, INDEX_CODE, REINFORCE, Step, StepState};
use crate::api;
use crate::prelude::*;
use crate::utilities::context::Ctx;

/// Apply every [`StepState::Pending`] step in `steps`, in place. Infallible
/// by construction: a step's error becomes its [`StepState::Failed`] rather
/// than the run's, which is what keeps one failure from hiding the others.
pub fn run(ctx: &mut Ctx<'_>, detected: &Detected, steps: &mut [Step]) {
    for step in steps.iter_mut() {
        if !step.state.is_pending() {
            continue;
        }
        step.state = match one(ctx, detected, step.id) {
            Ok(detail) => StepState::Applied { detail },
            Err(error) => StepState::Failed {
                error: error.to_string(),
            },
        };
    }
}

/// Apply one step, returning the line that describes what changed.
fn one(ctx: &mut Ctx<'_>, detected: &Detected, id: &str) -> Result<String> {
    match id {
        DATA_DIR => data_dir(ctx),
        AGENT_HOST => agent_host(ctx, detected),
        GIT_HOOKS => git_hooks(ctx, detected),
        INDEX_CODE => index_code(ctx, detected),
        REINFORCE => reinforce(ctx),
        // `cloud-auth` and `index-docs` are never planned Pending — they
        // are reports pointing at `comemory auth login` / `comemory index
        // <path>` — so they fall through to the refusal below, which also
        // guards any step a caller forces Pending by hand.
        other => Err(Error::Usage(format!(
            "step `{other}` is a report, not an action — setup never applies \
             it; see its `reason` for the command that does"
        ))),
    }
}

/// Create the data directory layout and open the database once so the schema
/// is migrated in.
fn data_dir(ctx: &mut Ctx<'_>) -> Result<String> {
    ctx.paths.ensure_dirs()?;
    ctx.conn()?;
    Ok(format!("initialized {}", ctx.paths.data_dir().display()))
}

/// Install the bundled skills and hooks into every host that needs them.
fn agent_host(ctx: &mut Ctx<'_>, detected: &Detected) -> Result<String> {
    let pending: Vec<&'static str> = detected
        .hosts_present
        .iter()
        .copied()
        .filter(|host| !detected.hosts_installed.contains(host))
        .collect();
    let mut installed = Vec::new();
    for host in pending {
        api::install::run(
            ctx,
            api::install::Request {
                host: host.to_string(),
                dry_run: false,
                config_dir: None,
            },
        )?;
        installed.push(host);
    }
    Ok(format!("installed for {}", installed.join(", ")))
}

/// Write the three reindex hooks. Never `force`: a foreign hook was already
/// reported unavailable at plan time, so reaching here means the slots are
/// free.
fn git_hooks(ctx: &mut Ctx<'_>, detected: &Detected) -> Result<String> {
    let repo = repo_root(detected)?;
    let resp = api::install_hooks::run(
        ctx,
        api::install_hooks::Request {
            repo: repo.clone(),
            force: false,
        },
    )?;
    Ok(format!("installed {} in {repo}", resp.installed.join(", ")))
}

/// Walk the repo and mirror its symbols into the code index.
fn index_code(ctx: &mut Ctx<'_>, detected: &Detected) -> Result<String> {
    let repo = detected
        .repo
        .as_ref()
        .ok_or_else(|| Error::Usage("index-code needs a git repository".into()))?;
    let resp = api::index_code::run(
        ctx,
        api::index_code::Request {
            repo: repo.label.clone(),
            path: repo.root.clone(),
            mode: api::index_code::IndexMode::default(),
        },
    )?;
    Ok(format!(
        "indexed {} file(s) in {}",
        resp.files_indexed, resp.repo
    ))
}

/// Turn on the config-backed search-edit reinforcement row.
fn reinforce(ctx: &mut Ctx<'_>) -> Result<String> {
    api::hooks::run(
        ctx,
        api::hooks::Request {
            repo: None,
            enable: Some(api::hooks::REINFORCE_HOOK.to_string()),
            disable: None,
        },
    )?;
    Ok("enabled in config.toml".to_string())
}

/// The detected repo root, or a usage error naming what is missing.
fn repo_root(detected: &Detected) -> Result<String> {
    detected
        .repo
        .as_ref()
        .map(|repo| repo.root.clone())
        .ok_or_else(|| Error::Usage("not a git repository".into()))
}

#[cfg(test)]
#[path = "tests/apply.rs"]
mod tests;
