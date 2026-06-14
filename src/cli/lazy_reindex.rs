//! Lazy auto-reindex: non-blocking background trigger shared by
//! `search-code` and `context`. Under [`AutoReindexMode::Lazy`], when the
//! command runs inside a git repo with a stale code index, spawn a DETACHED
//! `index-code` and return at once. Best-effort throughout: failures are
//! logged and swallowed so the search never blocks or fails. The cheap
//! staleness probe and the `schema_meta` debounce marker are documented on
//! [`should_reindex`] and [`maybe_trigger`].

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension};

use crate::config::paths::Paths;
use crate::config::{AutoReindexMode, Config};

/// `schema_meta` key prefix carrying the last lazy-reindex trigger marker
/// per repo. The stored value is `"<head>|<unix_millis>"`.
const TRIGGER_KEY_PREFIX: &str = "lazy_reindex_head:";

/// Resolved working-tree context for a lazy-reindex decision: the repo
/// label `index-code` will be invoked with and the absolute working-tree
/// root it will walk.
pub(crate) struct RepoContext {
    /// Repo label (the `--repo` filter, else the working-tree basename).
    pub repo: String,
    /// Absolute working-tree root (the `index-code --path` argument).
    pub root: PathBuf,
}

/// The last lazy-reindex trigger recorded for a repo: the HEAD it fired
/// for and the wall-clock millis it fired at. Parsed from the
/// `lazy_reindex_head:<repo>` marker. `pub` so the flat mirror test crate
/// can construct fixtures for [`should_reindex`].
pub struct LastTrigger {
    /// HEAD oid the previous trigger fired for.
    pub head: String,
    /// Unix-epoch milliseconds the previous trigger fired at.
    pub at_millis: u128,
}

/// Pure decision: trigger a lazy reindex now? `true` only for
/// [`AutoReindexMode::Lazy`] + stale (`last_indexed_head` differs from
/// `current_head`, or is `None` = never indexed) + not-debounced (no prior
/// trigger for this exact head, and the last trigger is older than
/// `threshold_ms` against `now_millis`). `Hook`/`Off` always return `false`.
/// Total and side-effect-free; `pub` for the external mirror test crate.
pub fn should_reindex(
    mode: &AutoReindexMode,
    current_head: &str,
    last_indexed_head: Option<&str>,
    last_trigger: Option<&LastTrigger>,
    now_millis: u128,
    threshold_ms: u64,
) -> bool {
    if !matches!(mode, AutoReindexMode::Lazy) {
        return false;
    }
    // Fresh: the index already reflects the current HEAD.
    if last_indexed_head == Some(current_head) {
        return false;
    }
    if let Some(trigger) = last_trigger {
        // Already fired a reindex for this exact HEAD — the spawned (or
        // in-flight) index-code will advance the cursor; don't pile on.
        if trigger.head == current_head {
            return false;
        }
        // Time-based debounce: a trigger younger than the threshold window
        // suppresses a fresh spawn even across a HEAD change, so a burst of
        // searches during a rebase cannot fork a herd of index-code procs.
        if now_millis.saturating_sub(trigger.at_millis) < u128::from(threshold_ms) {
            return false;
        }
    }
    true
}

/// Best-effort lazy reindex entry point for `search-code` / `context`.
///
/// Resolves the repo from the CWD, runs the cheap staleness probe (current
/// HEAD via one `git2` resolve vs `repo_marker.last_mined_commit` — no
/// working-tree walk, so uncommitted edits are intentionally not detected),
/// consults [`should_reindex`], records the `schema_meta` debounce marker
/// (`lazy_reindex_head:<repo>` = `"<head>|<unix_millis>"`), then spawns a
/// detached `index-code`. Every failure (no repo, unborn HEAD, marker or
/// spawn error) is logged and swallowed — the search proceeds against the
/// current (possibly slightly stale) index regardless.
pub(crate) fn maybe_trigger(
    conn: &Connection,
    cfg: &Config,
    paths: &Paths,
    repo_filter: Option<&str>,
) {
    if !matches!(cfg.indexing.auto_reindex, AutoReindexMode::Lazy) {
        return;
    }
    let Some(ctx) = repo_context(repo_filter) else {
        // Off-repo (no git repo at CWD) or bare repo: nothing to reindex.
        return;
    };
    let current_head = match crate::git_utils::current_head(&ctx.root) {
        Ok(h) => h,
        Err(e) => {
            tracing::debug!(error = %e, "lazy reindex: HEAD unresolved; skipping");
            return;
        }
    };
    let marker = read_repo_marker(conn, &ctx.repo);
    // Label/checkout collision guard: when the repo was already indexed from
    // a DIFFERENT working-tree root, the CWD is not that checkout (it just
    // reuses the label), so reindexing it would corrupt the foreign repo's
    // rows. Skip. A NULL/absent root (never indexed, or pre-v7) is allowed —
    // the never-indexed case must still be able to fire.
    if let Some(root) = marker.as_ref().and_then(|m| m.root_path.as_deref())
        && !same_root(root, &ctx.root)
    {
        tracing::debug!(
            repo = %ctx.repo,
            indexed_root = %root,
            cwd_root = %ctx.root.display(),
            "lazy reindex: CWD is not the indexed checkout for this label; skipping",
        );
        return;
    }
    let last_indexed = marker.and_then(|m| m.last_mined_commit);
    let last_trigger = read_last_trigger(conn, &ctx.repo);
    let now = now_millis();
    if !should_reindex(
        &cfg.indexing.auto_reindex,
        &current_head,
        last_indexed.as_deref(),
        last_trigger.as_ref(),
        now,
        cfg.indexing.auto_reindex_threshold_ms,
    ) {
        return;
    }
    // Record the trigger BEFORE spawning so a concurrent search in the
    // debounce window sees the marker even if the spawn is slow.
    record_trigger(conn, &ctx.repo, &current_head, now);
    spawn_index_code(&ctx, paths.data_dir());
}

/// Resolve the repo label + working-tree root from the process CWD, using
/// the same policy as [`crate::retrieval::code_rerank::WorkingSet::from_cwd`]:
/// discover the repo from the CWD, take the `--repo` filter as the label
/// (else the working-tree directory basename). Returns `None` off-repo, on
/// a bare repo (no workdir), or when the basename is not valid UTF-8.
pub(crate) fn repo_context(repo_filter: Option<&str>) -> Option<RepoContext> {
    let cwd = std::env::current_dir().ok()?;
    let git = git2::Repository::discover(&cwd).ok()?;
    let root = git.workdir()?.to_path_buf();
    let repo = match repo_filter {
        Some(r) => r.to_string(),
        None => root.file_name().and_then(|n| n.to_str())?.to_string(),
    };
    Some(RepoContext { repo, root })
}

/// The `repo_marker` columns the lazy probe needs: the HEAD at last index
/// (`last_mined_commit`) and the absolute working-tree root captured at
/// index time (`root_path`, NULL for never-indexed / pre-v7 repos).
struct RepoMarker {
    last_mined_commit: Option<String>,
    root_path: Option<String>,
}

/// Read the `repo_marker` row for `repo` (`last_mined_commit` + `root_path`)
/// in one query. `None` when there is no marker row (never indexed); read
/// errors also degrade to `None`.
fn read_repo_marker(conn: &Connection, repo: &str) -> Option<RepoMarker> {
    conn.query_row(
        "SELECT last_mined_commit, root_path FROM repo_marker WHERE repo = ?1",
        [repo],
        |r| {
            Ok(RepoMarker {
                last_mined_commit: r.get::<_, Option<String>>(0)?,
                root_path: r.get::<_, Option<String>>(1)?,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}

/// Whether the stored `indexed_root` denotes the same working tree as `cwd`.
/// Both are canonicalized so macOS `/var` vs `/private/var` symlinks and
/// trailing-slash differences do not produce a false mismatch; if either
/// fails to canonicalize (path vanished), fall back to a raw string compare.
fn same_root(indexed_root: &str, cwd_root: &Path) -> bool {
    let lhs = Path::new(indexed_root);
    match (lhs.canonicalize(), cwd_root.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => lhs == cwd_root,
    }
}

/// Read and parse the `lazy_reindex_head:<repo>` trigger marker. A missing
/// or malformed marker yields `None` (treated as "never triggered").
fn read_last_trigger(conn: &Connection, repo: &str) -> Option<LastTrigger> {
    let raw: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = ?1",
            [trigger_key(repo)],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()?;
    parse_trigger(&raw)
}

/// Parse a `"<head>|<unix_millis>"` trigger marker. Split out so the
/// encode/decode round-trip is unit-testable without a database. `pub`
/// for the external mirror test crate.
pub fn parse_trigger(raw: &str) -> Option<LastTrigger> {
    let (head, millis) = raw.split_once('|')?;
    if head.is_empty() {
        return None;
    }
    let at_millis = millis.parse::<u128>().ok()?;
    Some(LastTrigger {
        head: head.to_string(),
        at_millis,
    })
}

/// Encode a trigger marker as `"<head>|<unix_millis>"`. The inverse of
/// [`parse_trigger`]; shared with `record_trigger` so the writer and reader
/// cannot drift on the format. `pub` for the external mirror test crate.
pub fn encode_trigger(head: &str, at_millis: u128) -> String {
    format!("{head}|{at_millis}")
}

/// Upsert the `lazy_reindex_head:<repo>` marker. Best-effort: a write
/// failure is logged and swallowed (the worst case is a redundant spawn on
/// the next search, never a broken read path).
fn record_trigger(conn: &Connection, repo: &str, head: &str, now_millis: u128) {
    let value = encode_trigger(head, now_millis);
    if let Err(e) = conn.execute(
        "INSERT INTO schema_meta(key, value) VALUES(?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![trigger_key(repo), value],
    ) {
        tracing::debug!(error = %e, repo = %repo, "lazy reindex: trigger marker write failed");
    }
}

/// `schema_meta` key carrying the per-repo lazy-reindex trigger marker.
fn trigger_key(repo: &str) -> String {
    format!("{TRIGGER_KEY_PREFIX}{repo}")
}

/// Current wall-clock time in Unix-epoch milliseconds; `0` if the clock is
/// before the epoch (never, on a sane host). Folded into one helper so the
/// debounce and the marker timestamp share one source of "now".
fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Spawn a DETACHED `comemory index-code --repo <repo> --path <root>
/// --data-dir <dir>` and return immediately without awaiting it.
///
/// The child's stdio is redirected to null so it never writes to the
/// caller's terminal, and the [`std::process::Child`] handle is dropped
/// (not waited on) so the search returns without blocking on the index.
/// `--data-dir` pins the spawned reindex to the SAME store the search is
/// reading. Best-effort: a missing `current_exe` or a spawn failure is
/// logged via `tracing` and swallowed — a reindex that cannot start must
/// never surface as a search error.
fn spawn_index_code(ctx: &RepoContext, data_dir: &Path) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(error = %e, "lazy reindex: current_exe unavailable; skipping spawn");
            return;
        }
    };
    let mut cmd = Command::new(exe);
    cmd.arg("index-code")
        .arg("--repo")
        .arg(&ctx.repo)
        .arg("--path")
        .arg(&ctx.root)
        .arg("--data-dir")
        .arg(data_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    match cmd.spawn() {
        Ok(child) => {
            tracing::debug!(
                repo = %ctx.repo,
                pid = child.id(),
                "lazy reindex: detached index-code spawned",
            );
            // Drop the handle without waiting: the child runs to completion
            // independently and the OS reaps it (on unix it is reparented to
            // init once this process exits).
            drop(child);
        }
        Err(e) => {
            tracing::debug!(error = %e, repo = %ctx.repo, "lazy reindex: index-code spawn failed");
        }
    }
}
