//! The read-only half of `comemory setup`: probe what is already true about
//! this machine and this repo.
//!
//! Every fact comes from the command that owns it — `api::doctor` for the
//! data directory, `api::hooks` for the git hooks, `api::repos` for index
//! freshness, `sync::auth_file` for the credential — so a detected value can
//! never disagree with what that command reports on its own.
//!
//! **Never creates the database, never touches the network.** Every probe
//! that would open a connection is guarded on `comemory.db` already
//! existing, so a fresh data directory is left fresh; and the credential is
//! read from `auth.json` rather than by asking the platform.

use std::path::Path;

use super::RepoContext;
use crate::api::{self, Ctx};
use crate::git_utils;
use crate::prelude::*;

/// Everything `plan` needs to decide, gathered once.
#[derive(Debug, Clone)]
pub struct Detected {
    /// `true` when `comemory.db` exists and is writable.
    pub db_writable: bool,
    /// The applied schema version, or `"unknown"`.
    pub schema_version: String,
    /// The repo context, when the target path is a git working tree.
    pub repo: Option<RepoContext>,
    /// Git hooks reported installed by `api::hooks`, by name.
    pub hooks_installed: Vec<String>,
    /// Hook names whose `.git/hooks/` file exists but is not comemory's —
    /// installing over one needs `--force`, which setup never passes.
    pub hooks_foreign: Vec<String>,
    /// Whether search-edit reinforcement is enabled in `config.toml`.
    pub reinforce_enabled: bool,
    /// `api::repos` status for this repo: `fresh`, `stale`, `archived`,
    /// `unknown`, or `None` when the repo has never been indexed.
    pub index_status: Option<String>,
    /// Files changed since the last index, when the repo is `stale`.
    pub changed_files: Option<u64>,
    /// Registered document sources filed under this repo label.
    pub doc_sources: usize,
    /// Agent hosts whose CLI answered `--version`.
    pub hosts_present: Vec<&'static str>,
    /// Agent hosts whose bundle is already extracted for this version.
    pub hosts_installed: Vec<&'static str>,
    /// Whether a usable cloud credential was found.
    pub authenticated: bool,
}

/// Probe the machine and the repo at `target`.
///
/// `host_filter` restricts the agent-host probe to one already-validated
/// host name; `None` probes every host in [`api::install::HOSTS`].
///
/// # Errors
/// Propagates a genuine read failure from the delegated probes. A *missing*
/// thing is never an error — it is the absence this function exists to
/// report.
pub fn run(ctx: &mut Ctx<'_>, target: &Path, host_filter: Option<&str>) -> Result<Detected> {
    let repo = repo_context(target);
    let (db_writable, schema_version) = data_dir_state(ctx);
    let (hooks_installed, reinforce_enabled) = hooks(ctx, target)?;
    let hooks_foreign = foreign_hooks(target, &hooks_installed);
    let (index_status, changed_files) = index_state(ctx, repo.as_ref())?;
    let doc_sources = doc_sources(ctx, repo.as_ref())?;
    let hosts_present = hosts_present(host_filter);
    let hosts_installed = hosts_installed(ctx, &hosts_present);
    Ok(Detected {
        db_writable,
        schema_version,
        repo,
        hooks_installed,
        hooks_foreign,
        reinforce_enabled,
        index_status,
        changed_files,
        doc_sources,
        hosts_present,
        hosts_installed,
        // Read locally, never over the network: detection must not issue a
        // platform request just to answer "are you signed in?" (and
        // `cloud::device::org_status` would).
        authenticated: crate::sync::auth_file::AuthFile::load_usable(ctx.paths)?.is_some(),
    })
}

/// Whether the store is ready, and the schema version to show for it.
///
/// `api::doctor::run` is the authority on both, but it opens the database to
/// answer — which would *create* it. So it is consulted only once a
/// `comemory.db` exists; before that, "not initialized" is the honest answer
/// and no file is written.
///
/// A doctor error (a schema mismatch, most often) is converted rather than
/// propagated: setup exists to fix exactly that, and applying the `data-dir`
/// step opens the connection, which migrates. The message becomes the step's
/// detail, so nothing is swallowed.
fn data_dir_state(ctx: &mut Ctx<'_>) -> (bool, String) {
    if !ctx.paths.db_path().exists() {
        return (false, "not initialized".to_string());
    }
    match api::doctor::run(ctx, api::doctor::Request {}) {
        Ok(report) => (report.db_writable, report.schema_version),
        Err(error) => (false, error.to_string()),
    }
}

/// Resolve `target` into a [`RepoContext`], or `None` when it is not inside
/// a git working tree.
fn repo_context(target: &Path) -> Option<RepoContext> {
    let label = git_utils::repo_label_at(target)?;
    let root = std::path::absolute(target).ok()?;
    Some(RepoContext {
        label,
        root: root.display().to_string(),
        is_git: true,
    })
}

/// The installed git hooks and the `[reinforce]` toggle, both from
/// `api::hooks` so this cannot drift from `comemory hooks`.
fn hooks(ctx: &mut Ctx<'_>, target: &Path) -> Result<(Vec<String>, bool)> {
    let report = api::hooks::run(
        ctx,
        api::hooks::Request {
            repo: Some(target.display().to_string()),
            enable: None,
            disable: None,
        },
    )?;
    let mut installed = Vec::new();
    let mut reinforce = false;
    for row in report.hooks {
        if row.name == api::hooks::REINFORCE_HOOK {
            reinforce = row.installed;
        } else if row.installed {
            installed.push(row.name);
        }
    }
    Ok((installed, reinforce))
}

/// Hook names with a `.git/hooks/` file that `api::hooks` does *not* report
/// as installed — i.e. someone else's hook. `install-hooks` refuses to
/// clobber one without `--force`, so `plan` reports these as unavailable
/// rather than letting `apply` fail on them.
fn foreign_hooks(target: &Path, installed: &[String]) -> Vec<String> {
    let dir = git_utils::hooks_dir(target);
    api::hooks::GIT_HOOKS
        .iter()
        .filter(|name| !installed.iter().any(|i| i == *name) && dir.join(name).exists())
        .map(|name| (*name).to_string())
        .collect()
}

/// This repo's index freshness from `api::repos`, which answers with an
/// empty inventory (never a created database) when there is no `comemory.db`.
fn index_state(
    ctx: &mut Ctx<'_>,
    repo: Option<&RepoContext>,
) -> Result<(Option<String>, Option<u64>)> {
    let Some(repo) = repo else {
        return Ok((None, None));
    };
    let report = api::repos::run(
        ctx,
        api::repos::Request {
            repo: Some(repo.label.clone()),
        },
    )?;
    Ok(report
        .repos
        .into_iter()
        .next()
        .map_or((None, None), |row| (Some(row.status), row.changed_files)))
}

/// How many document sources are registered under this repo's label.
fn doc_sources(ctx: &mut Ctx<'_>, repo: Option<&RepoContext>) -> Result<usize> {
    let Some(repo) = repo else {
        return Ok(0);
    };
    // `api::sources::run` calls `Ctx::conn` unconditionally, so the
    // existence guard is what keeps a probe from creating the database.
    // `reconcile: false` keeps it a pure read as well — the default would
    // rewrite the SQLite mirror from `sources.toml`.
    if !ctx.paths.db_path().exists() {
        return Ok(0);
    }
    let rows = api::sources::run(ctx, api::sources::Request { reconcile: false })?;
    Ok(rows
        .iter()
        .filter(|source| source.repo.as_deref() == Some(repo.label.as_str()))
        .count())
}

/// Agent hosts whose CLI is on `PATH` and answers `--version`.
fn hosts_present(host_filter: Option<&str>) -> Vec<&'static str> {
    api::install::HOSTS
        .iter()
        .copied()
        .filter(|host| host_filter.is_none_or(|wanted| wanted == *host))
        .filter(|host| {
            std::process::Command::new(host)
                .arg("--version")
                .output()
                .is_ok_and(|out| out.status.success())
        })
        .collect()
}

/// Hosts whose bundle for *this* comemory version is already extracted.
/// Version-scoped on purpose: an upgrade should offer to reinstall.
fn hosts_installed(ctx: &Ctx<'_>, present: &[&'static str]) -> Vec<&'static str> {
    let bundle = ctx
        .paths
        .data_dir()
        .join("integrations")
        .join(env!("CARGO_PKG_VERSION"));
    if !bundle.exists() {
        return Vec::new();
    }
    present.to_vec()
}

#[cfg(test)]
#[path = "tests/detect.rs"]
mod tests;
