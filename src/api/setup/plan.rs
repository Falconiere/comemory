//! The pure half of `comemory setup`: turn a detected snapshot plus the
//! caller's selection into the step list.
//!
//! No I/O happens here, which is the point — every decision the wizard
//! appears to make is really made by [`run`], so the whole decision surface
//! is testable without a terminal.

use super::detect::Detected;
use super::{
    AGENT_HOST, CLOUD_AUTH, DATA_DIR, GIT_HOOKS, INDEX_CODE, INDEX_DOCS, REINFORCE, Request, Step,
    StepState,
};
use crate::api;

/// Build the plan. Steps come back in [`super::STEP_IDS`] order, every id
/// present exactly once, so a caller can index the list positionally and the
/// TTY summary never reorders between runs.
pub fn run(detected: &Detected, req: &Request) -> Vec<Step> {
    [
        data_dir(detected),
        agent_host(detected, req),
        git_hooks(detected),
        index_code(detected),
        index_docs(detected),
        reinforce(detected),
        cloud_auth(detected),
    ]
    .into_iter()
    .map(|step| deselect(step, req))
    .collect()
}

/// Apply `--only` / `--skip` to an already-planned step. `--skip` wins over
/// `--only` when both name the same id: deselection is the safe direction.
/// Only a `Pending` step can be deselected — a `Satisfied` or `Unavailable`
/// step has nothing to skip.
fn deselect(step: Step, req: &Request) -> Step {
    if !step.state.is_pending() {
        return step;
    }
    let only_excludes = !req.only.is_empty() && !req.only.iter().any(|id| id == step.id);
    let skipped = req.skip.iter().any(|id| id == step.id);
    if only_excludes || skipped {
        return Step {
            state: StepState::Skipped,
            ..step
        };
    }
    step
}

/// Shorthand for building a step.
fn step(id: &'static str, title: &str, detail: String, state: StepState) -> Step {
    Step {
        id,
        title: title.to_string(),
        detail,
        state,
    }
}

/// The data directory and its SQLite mirror.
fn data_dir(detected: &Detected) -> Step {
    let state = if detected.db_writable {
        StepState::Satisfied
    } else {
        StepState::Pending
    };
    let detail = if detected.db_writable {
        format!("schema {}", detected.schema_version)
    } else {
        "not initialized yet".to_string()
    };
    step(DATA_DIR, "Data directory", detail, state)
}

/// Bundled skills and hooks inside an agent host.
fn agent_host(detected: &Detected, req: &Request) -> Step {
    let title = "Agent skills + hooks";
    if detected.hosts_present.is_empty() {
        let wanted = req
            .host
            .clone()
            .unwrap_or_else(|| api::install::HOSTS.join(" or "));
        return step(
            AGENT_HOST,
            title,
            String::new(),
            StepState::Unavailable {
                reason: format!("no {wanted} CLI found on PATH"),
            },
        );
    }
    let pending: Vec<&str> = detected
        .hosts_present
        .iter()
        .copied()
        .filter(|host| !detected.hosts_installed.contains(host))
        .collect();
    if pending.is_empty() {
        return step(
            AGENT_HOST,
            title,
            format!("{} up to date", detected.hosts_installed.join(", ")),
            StepState::Satisfied,
        );
    }
    step(AGENT_HOST, title, pending.join(", "), StepState::Pending)
}

/// The three git reindex hooks.
fn git_hooks(detected: &Detected) -> Step {
    let title = "Git reindex hooks";
    let Some(repo) = &detected.repo else {
        return step(GIT_HOOKS, title, String::new(), not_a_repo());
    };
    if !detected.hooks_foreign.is_empty() {
        // `install-hooks` refuses to clobber a hand-written hook without
        // `--force`, and setup never passes it — so this is reported now,
        // as an unavailable step with the remedy, rather than surfacing as
        // an apply-time failure.
        return step(
            GIT_HOOKS,
            title,
            String::new(),
            StepState::Unavailable {
                reason: format!(
                    "a non-comemory {} hook exists; run `comemory install-hooks --force` to overwrite",
                    detected.hooks_foreign.join(", ")
                ),
            },
        );
    }
    let missing: Vec<&str> = crate::domains::code::hooks::GIT_HOOKS
        .iter()
        .copied()
        .filter(|name| !detected.hooks_installed.iter().any(|i| i == name))
        .collect();
    if missing.is_empty() {
        return step(
            GIT_HOOKS,
            title,
            format!("all three installed in {}", repo.label),
            StepState::Satisfied,
        );
    }
    step(GIT_HOOKS, title, missing.join(", "), StepState::Pending)
}

/// This repo's code index.
fn index_code(detected: &Detected) -> Step {
    let title = "Index this repo's code";
    let Some(repo) = &detected.repo else {
        return step(INDEX_CODE, title, String::new(), not_a_repo());
    };
    match detected.index_status.as_deref() {
        Some("archived") => step(
            INDEX_CODE,
            title,
            String::new(),
            StepState::Unavailable {
                reason: format!("repo `{}` is archived", repo.label),
            },
        ),
        Some("fresh") => step(
            INDEX_CODE,
            title,
            format!("{} is up to date", repo.label),
            StepState::Satisfied,
        ),
        Some("stale") => {
            let detail = detected.changed_files.map_or_else(
                || "HEAD moved since the last index".to_string(),
                |n| format!("{n} file(s) changed since the last index"),
            );
            step(INDEX_CODE, title, detail, StepState::Pending)
        }
        _ => step(
            INDEX_CODE,
            title,
            "never indexed".to_string(),
            StepState::Pending,
        ),
    }
}

/// Registered document sources. Never auto-selected: it needs a path the
/// operator chooses, so it is offered but not pending by default.
fn index_docs(detected: &Detected) -> Step {
    let title = "Index documents";
    let Some(repo) = &detected.repo else {
        return step(INDEX_DOCS, title, String::new(), not_a_repo());
    };
    if detected.doc_sources > 0 {
        return step(
            INDEX_DOCS,
            title,
            format!("{} source(s) registered", detected.doc_sources),
            StepState::Satisfied,
        );
    }
    step(
        INDEX_DOCS,
        title,
        format!("none registered for {}", repo.label),
        StepState::Unavailable {
            reason: "needs a path: run `comemory index <path>`".to_string(),
        },
    )
}

/// Search-edit auto-reinforcement.
fn reinforce(detected: &Detected) -> Step {
    let state = if detected.reinforce_enabled {
        StepState::Satisfied
    } else {
        StepState::Pending
    };
    let detail = if detected.reinforce_enabled {
        "enabled".to_string()
    } else {
        "learns which memories helped from what you edit after a search".to_string()
    };
    step(REINFORCE, "Search-edit reinforcement", detail, state)
}

/// Cloud sign-in — a report, never an action.
fn cloud_auth(detected: &Detected) -> Step {
    let title = "Cloud sync";
    if detected.authenticated {
        return step(
            CLOUD_AUTH,
            title,
            "signed in".to_string(),
            StepState::Satisfied,
        );
    }
    // Never pending, in any mode. The device flow needs a browser, a human,
    // and the daemon + initial-sync orchestration that `comemory auth login`
    // already owns; setup points at it rather than reimplementing it, so
    // this step is a report and never something that can fail.
    step(
        CLOUD_AUTH,
        title,
        String::new(),
        StepState::Unavailable {
            reason: "not signed in; run `comemory auth login`".to_string(),
        },
    )
}

/// The shared "this path is not a git working tree" state.
fn not_a_repo() -> StepState {
    StepState::Unavailable {
        reason: "not a git repository".to_string(),
    }
}

#[cfg(test)]
#[path = "tests/plan.rs"]
mod tests;
