//! When a lazy auto-reindex is due.
//!
//! The pure half of the lazy-reindex path: the staleness/debounce decision
//! ([`should_reindex`]), the `schema_meta` trigger marker it reads and writes
//! (`lazy_reindex_head:<repo>` = `"<head>|<unix_millis>"`), and the repo
//! label/working-tree resolution the decision needs. Split out of
//! `cli::lazy_reindex` by #167: deciding *whether* a repository's index is
//! stale is code-capability policy, while spawning the detached `index-code`
//! that acts on the decision stays a CLI responsibility.
//!
//! Every function here is total and free of process or terminal I/O.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::AutoReindexMode;
use crate::store::{self, Connection};

/// `schema_meta` key prefix carrying the last lazy-reindex trigger marker
/// per repo. The stored value is `"<head>|<unix_millis>"`.
const TRIGGER_KEY_PREFIX: &str = "lazy_reindex_head:";

/// Resolved working-tree context for a lazy-reindex decision: the repo
/// label `index-code` will be invoked with and the absolute working-tree
/// root it will walk.
pub(crate) struct RepoContext {
    /// Repo label (the `--repo` filter, else the main worktree's basename
    /// via `git_utils::repo_label`).
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

/// Resolve the repo label + working-tree root from the process CWD, using
/// the same policy as [`crate::retrieval::code_rerank::WorkingSet::from_cwd`]:
/// discover the repo from the CWD, take the `--repo` filter as the label,
/// else [`crate::domains::code::git_utils::repo_label`] — the main worktree's basename, so
/// a search from a linked worktree refreshes the main repo's index instead
/// of spawning an `index-code` under the worktree's directory name. `root`
/// stays the current checkout's working tree: that is where the files are.
/// Returns `None` off-repo, on a bare repo (no workdir), or when the
/// basename is not valid UTF-8.
pub(crate) fn repo_context(repo_filter: Option<&str>) -> Option<RepoContext> {
    let cwd = std::env::current_dir().ok()?;
    let git = git2::Repository::discover(&cwd).ok()?;
    let root = git.workdir()?.to_path_buf();
    let repo = match repo_filter {
        Some(r) => r.to_string(),
        None => crate::domains::code::git_utils::repo_label(&git)?,
    };
    Some(RepoContext { repo, root })
}

/// Read the `repo_marker` row for `repo` (`last_mined_commit` + `root_path` +
/// `archived`) in one query. `None` when there is no marker row (never
/// indexed); read errors also degrade to `None` — best-effort, like every
/// other probe in this module.
pub(crate) fn read_repo_marker(
    conn: &Connection,
    repo: &str,
) -> Option<store::repo_marker::LazyReindexMarker> {
    store::repo_marker::read_for_lazy_reindex(conn, repo)
        .ok()
        .flatten()
}

/// Whether the stored `indexed_root` denotes the same working tree as `cwd`.
/// Both are canonicalized so macOS `/var` vs `/private/var` symlinks and
/// trailing-slash differences do not produce a false mismatch; if either
/// fails to canonicalize (path vanished), fall back to a raw string compare.
pub(crate) fn same_root(indexed_root: &str, cwd_root: &Path) -> bool {
    let lhs = Path::new(indexed_root);
    match (lhs.canonicalize(), cwd_root.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => lhs == cwd_root,
    }
}

/// Read and parse the `lazy_reindex_head:<repo>` trigger marker. A missing
/// or malformed marker yields `None` (treated as "never triggered").
pub(crate) fn read_last_trigger(conn: &Connection, repo: &str) -> Option<LastTrigger> {
    let raw = store::schema_meta::get(conn, &trigger_key(repo))
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
pub(crate) fn record_trigger(conn: &Connection, repo: &str, head: &str, now_millis: u128) {
    let value = encode_trigger(head, now_millis);
    if let Err(e) = store::schema_meta::upsert(conn, &trigger_key(repo), &value) {
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
pub(crate) fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}
