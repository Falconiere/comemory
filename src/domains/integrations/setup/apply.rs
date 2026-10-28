//! The write half of `comemory setup`: run each pending step through the
//! command that already owns it.
//!
//! **A failing step never aborts the run.** The steps are independent
//! enablements, so one that fails is recorded as [`StepState::Failed`] and
//! the rest still run; the caller turns any failure into its exit code after
//! rendering the whole summary. Only a failure to *iterate* — which cannot
//! happen here — would propagate.

use super::detect::Detected;
use super::{AGENT_HOST, DATA_DIR, GIT_HOOKS, INDEX_CODE, REINFORCE, RepoContext, Step, StepState};
use crate::domains::integrations::install;
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
/// Each host is attempted even when an earlier one fails — the hosts are
/// independent — and a failure names both what was installed and what was
/// not, so a partial run is never reported as all-or-nothing.
fn agent_host(ctx: &mut Ctx<'_>, detected: &Detected) -> Result<String> {
    let pending: Vec<&'static str> = detected
        .hosts_present
        .iter()
        .copied()
        .filter(|host| !detected.hosts_installed.contains(host))
        .collect();
    let (mut installed, mut failed) = (Vec::new(), Vec::new());
    for host in pending {
        match install_host(ctx, host) {
            Ok(()) => installed.push(host),
            Err(error) => failed.push(format!("{host}: {error}")),
        }
    }
    install_summary(&installed, &failed)
}

/// Install the bundled skills and hooks into one host, for real.
fn install_host(ctx: &mut Ctx<'_>, host: &str) -> Result<()> {
    let request = install::Request {
        host: host.to_string(),
        dry_run: false,
        config_dir: None,
    };
    install::run(ctx, request).map(drop)
}

/// Fold the per-host outcomes into the step result: the "installed for ..."
/// line, or an unavailable error that also names every failed host.
fn install_summary(installed: &[&str], failed: &[String]) -> Result<String> {
    let done = if installed.is_empty() {
        "installed for no host".to_string()
    } else {
        format!("installed for {}", installed.join(", "))
    };
    if failed.is_empty() {
        Ok(done)
    } else {
        Err(Error::Unavailable(format!(
            "{done}; failed for {}",
            failed.join("; ")
        )))
    }
}

/// Write every reindex hook. Never `force`: a foreign hook was already
/// reported unavailable at plan time, so reaching here means the slots are
/// free.
fn git_hooks(ctx: &mut Ctx<'_>, detected: &Detected) -> Result<String> {
    use crate::domains::code::install_hooks::{Request, run};
    let root = &repo_context(detected, "not a git repository")?.root;
    let request = Request {
        repo: root.clone(),
        force: false,
    };
    run(ctx, request).map(|done| format!("installed {} in {root}", done.installed.join(", ")))
}

/// Walk the repo and mirror its symbols into the code index.
fn index_code(ctx: &mut Ctx<'_>, detected: &Detected) -> Result<String> {
    use crate::domains::code::index_code::{IndexMode, Request, run};
    let repo = repo_context(detected, "index-code needs a git repository")?;
    let (label, path) = (repo.label.clone(), repo.root.clone());
    let done = run(
        ctx,
        Request {
            repo: label,
            path,
            mode: IndexMode::default(),
        },
    )?;
    Ok(format!(
        "indexed {} file(s) in {}",
        done.files_indexed, done.repo
    ))
}

/// Turn on the config-backed search-edit reinforcement row.
fn reinforce(ctx: &mut Ctx<'_>) -> Result<String> {
    use crate::domains::code::hooks::{REINFORCE_HOOK, Request, run};
    let enable = Some(REINFORCE_HOOK.to_string());
    run(
        ctx,
        Request {
            repo: None,
            enable,
            disable: None,
        },
    )
    .map(|_| "enabled in config.toml".to_string())
}

/// The detected repo, or a usage error saying what is missing.
fn repo_context<'a>(detected: &'a Detected, missing: &str) -> Result<&'a RepoContext> {
    detected
        .repo
        .as_ref()
        .ok_or_else(|| Error::Usage(missing.into()))
}

#[cfg(test)]
#[path = "tests/apply.rs"]
mod tests;
