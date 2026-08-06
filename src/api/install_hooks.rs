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
use crate::git_utils::install_hook;
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

/// Body written to each hook file. The trailing `&` detaches the indexer so
/// git's hook runner returns immediately.
const SCRIPT: &str = "#!/usr/bin/env bash\n\
                      ROOT=\"$(git rev-parse --show-toplevel 2>/dev/null)\"\n\
                      [ -z \"$ROOT\" ] && exit 0\n\
                      REPO=\"$(basename \"$ROOT\")\"\n\
                      ( comemory index-code --repo \"$REPO\" --path \"$ROOT\" >/dev/null 2>&1 & )\n\
                      exit 0\n";

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
/// Pre-flight: verify every target hook either doesn't exist or `force` is
/// set BEFORE writing any of them, so a partial install (e.g. a fresh
/// `post-commit` next to an unchanged pre-existing `post-merge`) can never
/// happen.
pub fn run(_ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    let repo = PathBuf::from(&req.repo);
    if !req.force {
        for hook in HOOKS {
            let target = repo.join(".git").join("hooks").join(hook);
            if target.exists() {
                return Err(Error::Other(format!(
                    "{} already exists; pass --force to overwrite",
                    target.display()
                )));
            }
        }
    }
    for hook in HOOKS {
        install_hook(&repo, hook, SCRIPT)?;
    }
    Ok(Response {
        installed: HOOKS.iter().map(|h| h.to_string()).collect(),
        repo: req.repo,
    })
}
