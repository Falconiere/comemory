//! The `git` subprocess steps behind `api::memory_store::sync` — every
//! shell-out the `store-sync` job makes, and nothing else. Split out of
//! `memory_store.rs` when the conflict handling grew past the 300-line
//! ceiling; the in-process `git2` read probes stay in the parent, since they
//! must be cheap enough to run on every console poll.
//!
//! Every invocation goes through [`run`], which captures output with
//! `.output()` (never a detached `.spawn()`) so each step's stdout/stderr is
//! available for the job log and the error message, and which hardens the
//! child against blocking on a prompt: stdin is `/dev/null` and
//! `GIT_TERMINAL_PROMPT=0`, so a credential ask fails the step instead of
//! reading `/dev/tty` forever while the job holds the server's single write
//! permit. `GIT_SSH_COMMAND` is deliberately left to the operator — honoring
//! an existing value would need an env read outside `config/`, and
//! overriding it blindly would discard a custom key; `ssh -oBatchMode=yes`
//! is the matching setting for an unattended server.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::prelude::*;

/// The commit message every sync run writes. A fixed string: the job is a
/// mirror push of `memories/`, not an authored change.
const COMMIT_MESSAGE: &str = "comemory: sync memories";

/// One captured `git` invocation.
pub(super) struct GitRun {
    /// Whether git exited zero.
    pub(super) ok: bool,
    /// Captured stdout, lossily decoded.
    pub(super) stdout: String,
    /// Captured stderr, lossily decoded.
    pub(super) stderr: String,
}

impl GitRun {
    /// The most useful half of the output for an error message: stderr when
    /// git wrote any, else stdout (`git pull` reports conflicts on stdout).
    fn detail(&self) -> &str {
        let stderr = self.stderr.trim();
        if stderr.is_empty() {
            self.stdout.trim()
        } else {
            stderr
        }
    }

    /// Whether either stream carries git's `CONFLICT` marker.
    fn conflicted(&self) -> bool {
        self.stdout.contains("CONFLICT") || self.stderr.contains("CONFLICT")
    }

    /// This run when git exited zero, else an error naming the step — the
    /// shared "a failed git step fails the job" rule, in one place.
    pub(super) fn require(self, step: &str) -> Result<Self> {
        if self.ok {
            Ok(self)
        } else {
            Err(Error::Other(format!("{step} failed: {}", self.detail())))
        }
    }
}

/// Run one `git` subcommand in `root` with its output captured, logging the
/// command line first. `path`, when given, is appended as `-- <path>` so a
/// pathspec cannot be confused with a revision. Never `.spawn()`s without
/// waiting: `.output()` runs the child to completion, so a step cannot
/// outlive the job that started it. See the module doc for the prompt
/// hardening applied to every child.
///
/// `args` is `&[&'static str]`: every subcommand this module runs is
/// spelled out in the source, and the type says so, so a future caller
/// cannot reach argv with a runtime string through this door. The one
/// argument that is genuinely dynamic — the configured push remote — goes
/// through [`run_argv`], whose doc explains why it is safe there.
pub(super) fn run(
    root: &Path,
    args: &[&'static str],
    path: Option<&Path>,
    log: &impl Fn(&str),
) -> Result<GitRun> {
    run_argv(root, args, path, log)
}

/// [`run`] without the `'static` bound, for the single call that needs a
/// configured value in argv (`git push <remote> HEAD`). `Command::args`
/// passes each element to `execve` as one argument and never through a
/// shell, so a remote containing spaces, quotes or `;` is one argument,
/// not a second command — the reason this is a narrower door rather than a
/// hole in the one above.
fn run_argv(
    root: &Path,
    args: &[&str],
    path: Option<&Path>,
    log: &impl Fn(&str),
) -> Result<GitRun> {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0");
    if let Some(path) = path {
        cmd.arg("--").arg(path);
    }
    let target = path.map(|p| format!(" -- {}", p.display()));
    log(&format!(
        "git {}{}",
        args.join(" "),
        target.unwrap_or_default()
    ));
    let out = cmd.output()?;
    Ok(GitRun {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// `git pull --rebase --autostash`, with both conflict shapes caught.
///
/// A rebase that stops on `CONFLICT` exits non-zero; it is aborted (so the
/// work tree is left as it was found) and reported as an error naming every
/// conflicting path. An autostash whose re-apply conflicts is the sneakier
/// case: git exits ZERO for it ("Applying autostash resulted in conflicts"),
/// keeps the stash entry, and leaves the files unmerged in the work tree —
/// so the unmerged-path list is checked after EVERY pull, not only a failed
/// one. Because [`super::sync`] commits `memories/` before pulling, an
/// autostash conflict can only involve tracked files the operator changed
/// elsewhere in the repo; those are reported by path and left exactly as git
/// left them, never rolled back on the operator's behalf.
pub(super) fn pull(root: &Path, log: &impl Fn(&str)) -> Result<()> {
    let run = self::run(root, &["pull", "--rebase", "--autostash"], None, log)?;
    if !run.ok && !run.conflicted() {
        return Err(Error::Other(format!("git pull failed: {}", run.detail())));
    }
    let unmerged = unmerged_paths(root, log)?;
    if run.ok && unmerged.is_empty() {
        return Ok(());
    }
    log(&format!("git pull: {}", run.detail()));
    if !run.ok {
        let abort = self::run(root, &["rebase", "--abort"], None, log)?;
        if !abort.ok {
            log(&format!("git rebase --abort failed: {}", abort.detail()));
        }
    }
    Err(Error::Other(format!(
        "sync conflict in: {}",
        unmerged.join(", ")
    )))
}

/// Paths the index holds unmerged entries for — non-empty while a rebase
/// is stopped on a conflict, and after an autostash re-apply conflicted.
fn unmerged_paths(root: &Path, log: &impl Fn(&str)) -> Result<Vec<String>> {
    let out = run(root, &["diff", "--name-only", "--diff-filter=U"], None, log)?
        .require("git diff --diff-filter=U")?;
    Ok(out
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// Stage every change under `memories` and commit it, if there is any —
/// `Ok(true)` when a commit was made. `Ok(false)` when `git status
/// --porcelain -- <memories>` is empty: an already-synced store is a
/// successful no-op, not an error.
///
/// The commit is pathspec-limited to `memories/`, so a store that lives
/// inside a larger repo (a dotfiles checkout, say) never sweeps whatever
/// else the operator had staged into the sync commit — those entries stay
/// staged, untouched.
pub(super) fn commit_memories(root: &Path, memories: &Path, log: &impl Fn(&str)) -> Result<bool> {
    run(root, &["add", "-A"], Some(memories), log)?.require("git add")?;
    let status = run(root, &["status", "--porcelain"], Some(memories), log)?
        .require("git status --porcelain")?;
    if status.stdout.trim().is_empty() {
        log("skip: nothing to commit under memories/");
        return Ok(false);
    }
    run(root, &["commit", "-m", COMMIT_MESSAGE], Some(memories), log)?.require("git commit")?;
    Ok(true)
}

/// The full oid of HEAD.
pub(super) fn head(root: &Path, log: &impl Fn(&str)) -> Result<String> {
    let out = run(root, &["rev-parse", "HEAD"], None, log)?.require("git rev-parse HEAD")?;
    Ok(out.stdout.trim().to_string())
}

/// Push HEAD: `git push <remote> HEAD` when `remote` (the trimmed `[git]
/// remote`) is non-empty, so a configured target is honored even on a
/// branch with no upstream; else a bare `git push` to the upstream.
pub(super) fn push(root: &Path, remote: &str, log: &impl Fn(&str)) -> Result<()> {
    let args: &[&str] = if remote.is_empty() {
        &["push"]
    } else {
        &["push", remote, "HEAD"]
    };
    run_argv(root, args, None, log)?.require("git push")?;
    Ok(())
}
