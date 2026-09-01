//! `api::memory_store` — the one memory store's view and the `store-sync`
//! job (console-api spec §10, Non-Goal 3: "No multi-root memory stores. One
//! `data_dir` remains the model").
//!
//! The store is not a row in a table — it IS the data dir. [`list`] therefore
//! always answers exactly one [`Store`], keyed by the constant [`STORE_ID`],
//! and [`create`] is a hard [`Error::Unsupported`] rather than a silently
//! ignored request: a console that asks for a second root gets a `501` naming
//! the reason, not a fake success.
//!
//! **Conn-free.** Nothing here calls [`Ctx::conn`]: the counters come from the
//! filesystem and the sync state from `git2`, so asking about the store never
//! creates and migrates a `comemory.db` as a side effect (the same
//! must-not-create-the-db invariant `api::stats` documents).
//!
//! **`git` is the only subprocess.** [`sync`] shells out to `git` — and to
//! nothing else — always through the child module's `git::run`, which
//! captures output with `.output()` (never a detached `.spawn()`), so every
//! step's stdout/stderr is available for the job log and for the error
//! message, and which hardens each child against blocking on a credential
//! prompt. The read-side probes ([`list`]/[`get`]) use in-process `git2`
//! instead, since they must be cheap enough to run on every console poll.

mod git;

use std::path::{Path, PathBuf};

use git2::{BranchType, Oid, Repository, StatusOptions};
use serde::{Deserialize, Serialize};
use toml::Value;

use crate::api::Ctx;
use crate::config::patch::{patch_config_file, section};
use crate::config::{Config, Paths};
use crate::git_utils;
use crate::prelude::*;

/// The single store's id. Every route takes an `{id}` path segment so the
/// URL shape survives a future multi-store model, but today anything other
/// than this value is an honest `404`.
pub const STORE_ID: &str = "default";

/// Git state of the work tree the memory store lives in — the console's
/// "in sync / N ahead / dirty" strip. [`SyncState::default`] is exactly the
/// "the data dir is not in a repo" shape, which is why it is derived.
#[derive(Serialize, Debug, Default)]
pub struct SyncState {
    /// Whether `data_dir` sits inside a git work tree at all. When `false`
    /// every other field is `None` and [`sync`] refuses with a `400`.
    pub is_git_repo: bool,
    /// Short branch name of the checked-out HEAD; `None` when HEAD is
    /// detached or unborn (`git_utils::current_branch`).
    pub branch: Option<String>,
    /// Changed-or-untracked paths under `memories/`. `None` when the store is
    /// not in a repo, or when `memories/` cannot be expressed relative to the
    /// work tree root (the count would silently mean something else).
    pub dirty: Option<u64>,
    /// Commits HEAD is ahead of its upstream; `None` when there is no
    /// upstream to compare against.
    pub ahead: Option<usize>,
    /// Commits HEAD is behind its upstream; `None` under the same condition
    /// as [`SyncState::ahead`].
    pub behind: Option<usize>,
}

/// One memory store — today, the only one.
#[derive(Serialize, Debug)]
pub struct Store {
    /// Always [`STORE_ID`].
    pub id: String,
    /// Absolute path of the markdown source of truth (`<data_dir>/memories`).
    pub path: String,
    /// Push target: `[git] remote` when configured and non-empty, else the
    /// `origin` URL of the repo containing `data_dir`, else `None`.
    pub remote: Option<String>,
    /// `[git] auto_sync` — whether a save is expected to commit and push.
    pub push_on_save: bool,
    /// `*.md` files directly under `memories/` (`.trash/` excluded — the same
    /// rule `api::stats::Response::markdown_files` uses).
    pub markdown_files: u64,
    /// `*.md` files under `memories/.trash/` — soft-deleted memories.
    pub trashed_files: u64,
    /// Git state of the work tree — see [`SyncState`].
    pub sync: SyncState,
}

/// `POST /api/v1/memory-stores` body. Deserialized (and `deny_unknown_fields`
/// -checked) so the refusal is about the model, not about a typo, but never
/// acted on — see [`create`].
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    /// Requested root of the would-be second store.
    pub path: String,
    /// Requested push remote.
    #[serde(default)]
    pub remote: Option<String>,
    /// Requested `push_on_save` for the would-be store.
    #[serde(default)]
    pub push_on_save: Option<bool>,
}

/// `PATCH /api/v1/memory-stores/{id}` body: the `[git]` keys to rewrite.
/// Absent keys are left exactly as they are on disk — this is a patch, not a
/// replace, so a console that only knows about one toggle cannot silently
/// clear the other.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct PatchRequest {
    /// New `[git] auto_sync`.
    #[serde(default)]
    pub push_on_save: Option<bool>,
    /// New `[git] remote`. A non-empty name is what [`sync`] pushes to, as
    /// `git push <remote> HEAD`, whether or not the branch has an upstream.
    /// An empty string restores the git default (a bare `git push` to
    /// whatever the branch's upstream is), which is what `[git] remote = ""`
    /// already means.
    #[serde(default)]
    pub remote: Option<String>,
}

/// `POST /api/v1/memory-stores/{id}/sync` body — entirely optional.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct SyncRequest {
    /// Whether to push after committing. Absent falls back to `[git]
    /// auto_sync` ([`Store::push_on_save`]); an explicit value wins either
    /// way, so a console can push once without flipping the config.
    #[serde(default)]
    pub push: Option<bool>,
}

/// What one [`sync`] run did.
#[derive(Serialize, Debug)]
pub struct SyncResponse {
    /// Whether `git pull --rebase --autostash` ran (skipped when the branch
    /// has no upstream).
    pub pulled: bool,
    /// Whether anything under `memories/` was staged and committed.
    pub committed: bool,
    /// Whether the push ran: `git push <remote> HEAD` when `[git] remote` is
    /// set, else a bare `git push` to the upstream — skipped when there is
    /// neither.
    pub pushed: bool,
    /// HEAD after the run, when a commit was made — the sync commit as it
    /// stands AFTER the pull's rebase (the id that was pushed), which is why
    /// it can differ from the id `git commit` first minted.
    pub commit: Option<String>,
    /// Always empty on success: a conflicting rebase is aborted and reported
    /// as an error naming the paths, never as a partial success.
    pub conflicts: Vec<String>,
}

/// Every memory store — exactly one, always.
pub fn list(ctx: &mut Ctx<'_>) -> Result<Vec<Store>> {
    Ok(vec![build(ctx.paths, ctx.cfg)?])
}

/// One store by id. Any id other than [`STORE_ID`] is [`Error::NotFound`].
pub fn get(ctx: &mut Ctx<'_>, id: &str) -> Result<Store> {
    ensure_id(id)?;
    build(ctx.paths, ctx.cfg)
}

/// Write the supplied `[git]` keys into `config.toml` and answer the store as
/// the file now says. Only the keys present in `req` are touched, through the
/// one shared [`patch_config_file`] primitive (so the atomic tmp+rename and
/// the "key exists but is not a table" refusal cannot drift), and the returned
/// [`Store`] is rendered from a clone of the live config with those values
/// applied. The HTTP route does not answer with it: it reloads `AppState.cfg`
/// and re-reads through [`get`], so an env override the reload re-applies
/// (`COMEMORY_GIT_AUTO_SYNC`) shows in the body exactly as the next `GET`
/// would report it.
pub fn patch(ctx: &mut Ctx<'_>, id: &str, req: &PatchRequest) -> Result<Store> {
    ensure_id(id)?;
    let mut cfg = ctx.cfg.clone();
    if let Some(push_on_save) = req.push_on_save {
        cfg.git.auto_sync = push_on_save;
    }
    if let Some(remote) = &req.remote {
        cfg.git.remote.clone_from(remote);
    }
    let cfg = cfg.validate()?;
    patch_config_file(&ctx.paths.config_file(), |root| {
        let git = section(root, "git")?;
        if let Some(push_on_save) = req.push_on_save {
            git.insert("auto_sync".into(), Value::Boolean(push_on_save));
        }
        if let Some(remote) = &req.remote {
            git.insert("remote".into(), Value::String(remote.clone()));
        }
        Ok(())
    })?;
    build(ctx.paths, &cfg)
}

/// Refuse to create a second store (spec Non-Goal 3). The request is taken by
/// value and dropped: there is no partial support to fall back to, and a
/// `501` naming the reason is more useful than a `404`.
pub fn create(_req: CreateRequest) -> Result<Store> {
    Err(Error::Unsupported(
        "comemory has one memory store (data_dir/memories); a second root is not modelled".into(),
    ))
}

/// Commit, pull, and (optionally) push `memories/` — the `store-sync` job.
///
/// The order is commit → pull → push, deliberately. `git pull --rebase
/// --autostash` stashes TRACKED changes only, so pulling first with an
/// untracked new memory whose path also arrives from upstream fails with
/// "untracked working tree files would be overwritten" — no `CONFLICT`
/// marker, so a generic pull error, on every retry. Committing `memories/`
/// first turns that case into either a clean rebase (an identical file is
/// skipped as already upstream) or a real `CONFLICT`, which `git::pull`
/// aborts and reports by path. The response fields keep their meaning:
/// `committed`/`commit` describe the sync commit, `pulled` whether the
/// rebase ran, `pushed` whether the push ran.
///
/// `log` receives one line per `git` invocation and per skipped step; the HTTP
/// route feeds it into the job's log tail / SSE `log` stream.
pub fn sync(
    ctx: &mut Ctx<'_>,
    id: &str,
    req: &SyncRequest,
    log: impl Fn(&str),
) -> Result<SyncResponse> {
    ensure_id(id)?;
    let root = work_tree(ctx.paths.data_dir())?;
    let memories = ctx.paths.memories_dir();
    let upstream = upstream_target(&Repository::open(&root)?)?.is_some();
    let committed = git::commit_memories(&root, &memories, &log)?;
    // Both skipped steps are visible in the response (`pulled`/`pushed`) and
    // in the absence of their `git …` log line, so neither logs a skip.
    let pulled = upstream;
    if pulled {
        git::pull(&root, &log)?;
    }
    let commit = if committed {
        Some(git::head(&root, &log)?)
    } else {
        None
    };
    let remote = ctx.cfg.git.remote.trim();
    let pushed = req.push.unwrap_or(ctx.cfg.git.auto_sync) && (upstream || !remote.is_empty());
    if pushed {
        git::push(&root, remote, &log)?;
    }
    Ok(SyncResponse {
        pulled,
        committed,
        pushed,
        commit,
        conflicts: Vec::new(),
    })
}

/// The one store's current view under `cfg`. Takes the config explicitly (not
/// through the `Ctx`) so [`patch`] can render the store the caller is ABOUT to
/// have — the server reloads `AppState.cfg` after the write, but the response
/// must already reflect it.
fn build(paths: &Paths, cfg: &Config) -> Result<Store> {
    let data_dir = paths.data_dir();
    let memories = paths.memories_dir();
    let configured = cfg.git.remote.trim();
    let remote = if configured.is_empty() {
        git_utils::remote_url(data_dir, "origin")
    } else {
        Some(configured.to_string())
    };
    Ok(Store {
        id: STORE_ID.to_string(),
        path: memories.to_string_lossy().into_owned(),
        remote,
        push_on_save: cfg.git.auto_sync,
        markdown_files: count_markdown(&memories),
        trashed_files: count_markdown(&paths.trash_dir()),
        sync: sync_state(data_dir, &memories)?,
    })
}

/// `Ok(())` only for [`STORE_ID`].
fn ensure_id(id: &str) -> Result<()> {
    if id == STORE_ID {
        Ok(())
    } else {
        Err(Error::NotFound(format!("memory store not found: {id}")))
    }
}

/// `*.md` files directly under `dir`, every subdirectory excluded. A missing
/// directory counts as zero: a fresh data dir is a legitimate state, not an
/// error.
fn count_markdown(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .count() as u64
}

/// Git state of the work tree containing `data_dir`. A data dir outside any
/// repo is the documented `is_git_repo: false` shape; a repo that *is* found
/// but then fails a probe propagates, so a broken store is visible rather
/// than reported as clean.
fn sync_state(data_dir: &Path, memories: &Path) -> Result<SyncState> {
    let Ok(repo) = Repository::discover(data_dir) else {
        return Ok(SyncState::default());
    };
    let head = repo.head().ok().and_then(|h| h.target());
    let (ahead, behind) = match (head, upstream_target(&repo)?) {
        (Some(local), Some(upstream)) => {
            let counts = repo.graph_ahead_behind(local, upstream)?;
            (Some(counts.0), Some(counts.1))
        }
        _ => (None, None),
    };
    Ok(SyncState {
        is_git_repo: true,
        branch: git_utils::current_branch(data_dir)?,
        dirty: dirty_count(&repo, memories)?,
        ahead,
        behind,
    })
}

/// The oid of HEAD's upstream branch, or `None` when HEAD is unborn,
/// detached, or has no upstream configured — the three "nothing to compare
/// against" states, none of which is a failure.
fn upstream_target(repo: &Repository) -> Result<Option<Oid>> {
    let branch = repo
        .head()
        .ok()
        .filter(git2::Reference::is_branch)
        .and_then(|head| head.shorthand().ok().map(str::to_string));
    let Some(branch) = branch else {
        return Ok(None);
    };
    let Ok(upstream) = repo.find_branch(&branch, BranchType::Local)?.upstream() else {
        return Ok(None);
    };
    Ok(upstream.get().target())
}

/// Changed-or-untracked paths under `memories`, or `None` when that directory
/// cannot be expressed relative to the work tree root. Both sides are
/// canonicalized before the strip, so a symlinked root (macOS `/var` →
/// `/private/var`) still matches.
fn dirty_count(repo: &Repository, memories: &Path) -> Result<Option<u64>> {
    let Some(rel) = repo
        .workdir()
        .and_then(|w| w.canonicalize().ok())
        .zip(memories.canonicalize().ok())
        .and_then(|(workdir, dir)| dir.strip_prefix(&workdir).ok().map(Path::to_path_buf))
    else {
        return Ok(None);
    };
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .pathspec(&rel);
    Ok(Some(repo.statuses(Some(&mut opts))?.len() as u64))
}

/// The work tree root containing `data_dir`. A data dir outside any repo — or
/// inside a bare one, which has no work tree to commit into — is the
/// documented `400` for every sync attempt.
fn work_tree(data_dir: &Path) -> Result<PathBuf> {
    let repo = Repository::discover(data_dir).map_err(|e| {
        tracing::debug!(error = %e, "memory store: no git repository found");
        Error::BadRequest("memory store is not a git repository".into())
    })?;
    repo.workdir()
        .map(Path::to_path_buf)
        .ok_or_else(|| Error::BadRequest("memory store is not a git repository".into()))
}

#[cfg(test)]
#[path = "tests/memory_store.rs"]
mod tests;
