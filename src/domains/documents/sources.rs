//! `domains::documents::sources::{Request, run}` — the shared middle of
//! `comemory sources` / `GET /api/v1/sources`: optionally reconcile the SQLite
//! `source_roots` mirror against `sources.toml`, then list every
//! registered source with its per-status file counts. Moved out of
//! `cli::sources::run` (Binding Rule 1).
//!
//! `reconcile` is split out of the CLI's always-on behavior so a
//! `--read-only` HTTP server can list without the mirror-write side effect
//! (§Security "Read-only side-effect degradation"): the CLI always passes
//! `true`; the HTTP route passes `false` on a `--read-only` server.

use serde::{Deserialize, Serialize};

use crate::domains::documents::source::mirror;
use crate::domains::documents::source::registry::Registry;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::{document_share, repo_marker, repository_approval, sources};
use crate::utilities::context::Ctx;

/// `comemory sources` / `GET /api/v1/sources` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Reconcile the SQLite mirror against `sources.toml` before listing.
    #[serde(default = "default_reconcile")]
    pub reconcile: bool,
}

fn default_reconcile() -> bool {
    true
}

/// One `comemory sources` / `GET /api/v1/sources` row.
#[derive(Serialize, Debug)]
pub struct Row {
    /// 32-hex-char source id.
    pub id: String,
    /// Absolute, symlink-resolved source path.
    pub canonical_path: String,
    /// `"file"` or `"dir"`.
    pub kind: String,
    /// Optional repository label.
    pub repo: Option<String>,
    /// `"active"` or `"unreachable"`.
    pub status: String,
    /// Files indexed successfully.
    pub indexed: usize,
    /// Files that failed extraction/indexing.
    pub error: usize,
    /// Files flagged stale (content changed since the last index).
    pub stale: usize,
    /// Row last-update timestamp.
    pub last_checked: String,
    /// Documents under this source that are withheld from sharing, as
    /// `(repository-relative path, rule)`. Empty when nothing is withheld —
    /// an operator has to be able to see WHY a document stayed local.
    pub withheld: Vec<(String, String)>,
    /// Canonical repository every document under this source is shared as,
    /// or `None` when none of them is shared at all.
    pub shared_as: Option<String>,
    /// Why nothing under this source is shared, when nothing is. `None` when
    /// it IS shared. Four things can be missing and an operator cannot tell
    /// them apart from the counts, so each says which one it was.
    pub unshared_reason: Option<String>,
}

/// List every registered source, reconciling the SQLite mirror against
/// `sources.toml` first when `req.reconcile` is set — so the report
/// reflects the durable source of truth even when the mirror fell behind
/// (e.g. a hand-edited `sources.toml`, or a run of `comemory
/// index`/`unindex` from another process).
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Vec<Row>> {
    let registry = Registry::new(ctx.paths.clone());
    let conn: &Connection = ctx.conn()?;
    if req.reconcile {
        mirror::reconcile(conn, &registry.load()?)?;
    }
    let mut rows = Vec::new();
    let policy_loaded = repository_approval::len(conn)? > 0;
    for root in sources::list(conn)? {
        let counts = sources::file_status_counts(conn, &root.id)?;
        let id = root.id.clone();
        let (shared_as, unshared_reason) =
            match sharing_of(conn, root.repo.as_deref(), policy_loaded)? {
                Ok(repo) => (Some(repo), None),
                Err(reason) => (None, Some(reason)),
            };
        rows.push(Row {
            id: root.id,
            canonical_path: root.canonical_path,
            kind: root.kind,
            repo: root.repo,
            status: root.status,
            indexed: counts.indexed,
            error: counts.error,
            stale: counts.stale,
            last_checked: root.updated_at,
            withheld: document_share::blocked_for_source(conn, &id)?,
            shared_as,
            unshared_reason,
        });
    }
    Ok(rows)
}

/// What this source's label resolves to for sharing, or the reason it does
/// not. The same four refusals the capture applies, reported rather than
/// logged — a document that stayed local for a fixable reason (an unapproved
/// repository, a repository never indexed here) is only fixable if an
/// operator can see which reason it was.
fn sharing_of(
    conn: &Connection,
    label: Option<&str>,
    policy_loaded: bool,
) -> Result<std::result::Result<String, String>> {
    let Some(label) = label.map(str::trim).filter(|l| !l.is_empty()) else {
        return Ok(Err("no repository label".to_string()));
    };
    if !policy_loaded {
        return Ok(Err("no sync policy has been loaded".to_string()));
    }
    let Some(canonical) = repository_approval::canonical_for(conn, label)? else {
        return Ok(Err(format!("repository `{label}` is not approved")));
    };
    if repo_marker::root_path(conn, label)?.is_none() {
        return Ok(Err(format!(
            "repository `{label}` has no indexed root on this machine"
        )));
    }
    Ok(Ok(canonical))
}

#[cfg(test)]
#[path = "tests/sources.rs"]
mod tests;
