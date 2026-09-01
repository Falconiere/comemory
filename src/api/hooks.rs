//! `api::hooks::{Request, Response, run}` — the shared middle of `comemory
//! hooks` / `GET|POST /api/v1/hooks`: read and per-hook toggle the reindex
//! git hooks `install-hooks` writes, plus a fourth, config-backed row for
//! search→edit auto-reinforcement. Moved out of `cli::hooks::run` (Binding
//! Rule 1).
//!
//! Two independent state stores, one read/write surface:
//! * `post-commit` / `post-merge` / `post-checkout` — state lives on disk in
//!   `.git/hooks/` (no DB table): [`git_utils::hook_installed`] reads it,
//!   [`git_utils::remove_hook`] / [`git_utils::install_hook`] write it.
//! * `search-edit-reinforcement` — state lives in `config.toml`'s
//!   `[reinforce]` section ([`crate::config::ReinforceConfig::enabled`]).
//!
//! `install-hooks` (`api::install_hooks`) is unchanged by this module — it
//! remains the install-all-three shorthand.
//!
//! Conn-free, like `api::install_hooks` — `run` never calls [`Ctx::conn`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::git_utils;
use crate::prelude::*;

/// The three git hooks `install-hooks` writes, each independently
/// controllable here via `--enable`/`--disable`.
pub const GIT_HOOKS: &[&str] = &["post-commit", "post-merge", "post-checkout"];

/// The fourth, config-backed row: search→edit auto-reinforcement
/// (`[reinforce]` in `config.toml`), reported and toggled through this same
/// surface even though its state isn't a `.git/hooks/` file.
pub const REINFORCE_HOOK: &str = "search-edit-reinforcement";

/// `comemory hooks` / `GET|POST /api/v1/hooks` request.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Repo root the three git hooks are read from / written to. Defaults
    /// to the current working directory. Irrelevant to the
    /// `search-edit-reinforcement` row.
    #[serde(default)]
    pub repo: Option<String>,
    /// Hook name to install/enable (one of [`GIT_HOOKS`] or
    /// [`REINFORCE_HOOK`]).
    #[serde(default)]
    pub enable: Option<String>,
    /// Hook name to remove/disable (one of [`GIT_HOOKS`] or
    /// [`REINFORCE_HOOK`]).
    #[serde(default)]
    pub disable: Option<String>,
}

/// One row of `comemory hooks`' report.
#[derive(Serialize, Debug)]
pub struct HookRow {
    /// The hook name (one of [`GIT_HOOKS`] or [`REINFORCE_HOOK`]).
    pub name: String,
    /// Whether it is currently active.
    pub installed: bool,
    /// Where `installed` was determined from: `"git"` (a `.git/hooks/` file
    /// carrying comemory's marker) or `"config"` (`config.toml`'s
    /// `[reinforce]` section).
    pub source: &'static str,
}

/// `comemory hooks` / `GET|POST /api/v1/hooks` response: one row per hook,
/// always in [`GIT_HOOKS`] order followed by [`REINFORCE_HOOK`].
#[derive(Serialize, Debug)]
pub struct Response {
    /// The four rows.
    pub hooks: Vec<HookRow>,
}

/// Apply at most one `enable` and one `disable` (validated against the
/// known hook names before either runs, so a typo in `--disable` cannot
/// leave a half-applied `--enable` behind), then report all four rows.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    let repo = PathBuf::from(req.repo.as_deref().unwrap_or("."));
    if let Some(name) = req.enable.as_deref() {
        known_hook(name)?;
    }
    if let Some(name) = req.disable.as_deref() {
        known_hook(name)?;
    }

    let mut reinforce_enabled = ctx.cfg.reinforce.enabled;
    if let Some(name) = req.enable.as_deref() {
        if name == REINFORCE_HOOK {
            set_reinforce_enabled(&ctx.paths.config_file(), true)?;
            reinforce_enabled = true;
        } else {
            git_utils::install_hook(&repo, name, git_utils::REINDEX_HOOK_SCRIPT)?;
        }
    }
    if let Some(name) = req.disable.as_deref() {
        if name == REINFORCE_HOOK {
            set_reinforce_enabled(&ctx.paths.config_file(), false)?;
            reinforce_enabled = false;
        } else {
            git_utils::remove_hook(&repo, name)?;
        }
    }

    let mut hooks: Vec<HookRow> = GIT_HOOKS
        .iter()
        .map(|name| HookRow {
            name: (*name).to_string(),
            installed: git_utils::hook_installed(&repo, name),
            source: "git",
        })
        .collect();
    hooks.push(HookRow {
        name: REINFORCE_HOOK.to_string(),
        installed: reinforce_enabled,
        source: "config",
    });
    Ok(Response { hooks })
}

/// `Ok(())` when `name` is one of [`GIT_HOOKS`] or [`REINFORCE_HOOK`], else
/// `Err(Error::Usage)` naming the unknown value.
fn known_hook(name: &str) -> Result<()> {
    if GIT_HOOKS.contains(&name) || name == REINFORCE_HOOK {
        Ok(())
    } else {
        Err(Error::Usage(format!(
            "unknown hook `{name}` (expected one of post-commit, post-merge, \
             post-checkout, {REINFORCE_HOOK})"
        )))
    }
}

/// Toggle `[reinforce] enabled` in `config.toml`, preserving every other
/// key — a raw `toml::Value` patch (same technique as
/// `eval::tune::apply_to_config_file`, scoped to the one boolean this
/// command owns) rather than a full typed round-trip, so an operator's
/// hand-edited comments and unrelated sections survive. Creates the file
/// (and its `[reinforce]` table) if missing, so the very first
/// `--disable`/`--enable` on a fresh install still round-trips.
fn set_reinforce_enabled(path: &Path, enabled: bool) -> Result<()> {
    let mut root: toml::Value = if path.exists() {
        let raw = std::fs::read_to_string(path)?;
        toml::from_str(&raw).map_err(|e| Error::Config(format!("config.toml: {e}")))?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let table = root
        .as_table_mut()
        .ok_or_else(|| Error::Config("config.toml: root is not a table".into()))?;
    let reinforce = table
        .entry("reinforce")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| Error::Config("config.toml: [reinforce] is not a table".into()))?;
    reinforce.insert("enabled".into(), toml::Value::Boolean(enabled));
    let rendered = toml::to_string_pretty(&root)
        .map_err(|e| Error::Config(format!("config.toml render: {e}")))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, rendered)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/hooks.rs"]
mod tests;
