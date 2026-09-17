//! `api::install_hooks::{Request, run}` — the shared middle of `comemory
//! install-hooks` / `POST /api/v1/hooks/install`: drop git hooks into a repo
//! so commits/merges/checkouts kick off a background `comemory index-code`.
//! Moved out of `cli::install_hooks::run` (Binding Rule 1).
//!
//! Conn-free — `run` never calls [`Ctx::conn`]; `&mut Ctx` is threaded only
//! for signature uniformity with every other `api::<cmd>::run`.
//!
//! **Containment is not this file's job.** The HTTP route handler
//! canonicalizes and contains `req.repo` (`security::contain_abs`) before
//! calling [`run`]; this middle stays exactly as unrestricted as the CLI.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::git_utils::{self, install_hook};
use crate::prelude::*;

/// `comemory install-hooks` / `POST /api/v1/hooks/install` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Repo root to install hooks into. Defaults to the current working
    /// directory (matches the CLI's `default_value = "."`).
    #[serde(default = "default_repo")]
    pub repo: String,
    /// Overwrite existing hook files.
    #[serde(default)]
    pub force: bool,
}

/// The CLI's `--repo` default (`"."`).
fn default_repo() -> String {
    ".".to_string()
}

/// Hooks installed on every call: `post-commit`, `post-merge`,
/// `post-checkout`.
const HOOKS: &[&str] = &["post-commit", "post-merge", "post-checkout"];

/// Report of one `install-hooks` run.
#[derive(Serialize, Debug)]
pub struct Response {
    /// Hooks written (always all of [`HOOKS`] on success).
    pub installed: Vec<String>,
    /// The repo root hooks were installed into.
    pub repo: String,
}

/// Install (or, with `req.force`, overwrite) the three reindex hooks.
///
/// Pre-flight: verify every target hook is writable BEFORE writing any of
/// them, so a partial install (e.g. a fresh `post-commit` next to an
/// untouched pre-existing `post-merge`) can never happen.
///
/// A target is writable when it does not exist, when `force` is set, or when
/// it is **our own** hook — one carrying [`git_utils::HOOK_MARKER`].
/// Rewriting a comemory-written hook with the body this binary ships is
/// idempotent when it already matches and a repair when it does not
/// ([`git_utils::hook_outdated`]), and that repair is the point: hooks
/// written before the worktree-label rule pass
/// `basename "$(git rev-parse --show-toplevel)"` as `--repo`, so every commit
/// and every `git worktree add` in the repo mints a `<worktree-dir>` repo
/// label. Nothing replaced them, because [`git_utils::hook_installed`]
/// matches on the marker alone and reported them as installed.
///
/// `--force` keeps its one remaining job: clobbering a **foreign** hook,
/// which is somebody else's file and never ours to overwrite silently.
pub fn run(_ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    let repo = PathBuf::from(&req.repo);
    if !req.force {
        for hook in HOOKS {
            let target = git_utils::hooks_dir(&repo).join(hook);
            if target.exists() && !git_utils::hook_installed(&repo, hook) {
                return Err(Error::Other(format!(
                    "{} already exists and was not written by comemory; \
                     pass --force to overwrite",
                    target.display()
                )));
            }
        }
    }
    for hook in HOOKS {
        install_hook(&repo, hook, git_utils::REINDEX_HOOK_SCRIPT)?;
    }
    Ok(Response {
        installed: HOOKS.iter().map(ToString::to_string).collect(),
        repo: req.repo,
    })
}
