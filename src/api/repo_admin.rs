//! `api::repo_admin` — `POST /api/v1/repos`, `PATCH /api/v1/repos/{name}`,
//! `POST /api/v1/repos/{name}/archive`, `DELETE /api/v1/repos/{name}`
//! (console-api spec §10).
//!
//! The four repo-administration cores, all of them thin writers over
//! `repo_marker` (plus [`crate::store::repo_drop`] for the destructive
//! one). There is no CLI counterpart: a repo becomes "connected" on the CLI
//! by running `comemory index-code` at it, so these exist to give the
//! console the same lifecycle explicitly — connect (register the root
//! before anything is indexed), patch (move the root), archive (stop
//! indexing, keep the memories), disconnect (drop the code index, keep the
//! memories). Only archive actually STOPS indexing: disconnect removes the
//! `repo_marker` row, so under the default lazy auto-reindex the next
//! `search-code` / `context` run from that checkout rebuilds the index.
//!
//! Path containment is the caller's job: every route contains `root`
//! against `AppState::allowed_roots` BEFORE calling in here (§Security
//! "Path containment"), exactly as `POST /api/v1/code/index` does.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::prelude::*;
use crate::store::{code_row, repo_drop};

/// `POST /api/v1/repos` request — register a working-tree root under a repo
/// label.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ConnectRequest {
    /// Absolute working-tree root. Contained by the route first, then
    /// canonicalized here.
    pub root: String,
    /// The label to file this root under. Defaults to `root`'s basename —
    /// the same rule `cli::lazy_reindex::repo_context` uses.
    #[serde(default)]
    pub repo: Option<String>,
    /// Start an `index-code` job right away. Honored by the ROUTE (which
    /// owns the job registry), not by [`connect`]: the core only writes the
    /// marker row.
    #[serde(default)]
    pub index_now: bool,
}

/// `POST /api/v1/repos` response.
#[derive(Serialize, Debug)]
pub struct ConnectResponse {
    /// The label the root was registered under.
    pub repo: String,
    /// The canonicalized root, as stored in `repo_marker.root_path`.
    pub root_path: String,
    /// The `index-code` job started for `index_now`, filled in by the route
    /// after [`connect`] returns; absent when no job was started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}

/// `PATCH /api/v1/repos/{name}` request. Only `root` is modelled; the other
/// four fields exist solely so a client naming one gets an explicit `501
/// unsupported` instead of a silently-ignored field (Non-Goal 4).
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct PatchRequest {
    /// Move the repo's working-tree root. Contained by the route first.
    #[serde(default)]
    pub root: Option<String>,
    /// Rename the label — unsupported.
    #[serde(default)]
    pub name: Option<String>,
    /// Pin a branch — unsupported.
    #[serde(default)]
    pub branch: Option<String>,
    /// Per-repo include globs — unsupported.
    #[serde(default)]
    pub include: Option<Vec<String>>,
    /// Per-repo exclude globs — unsupported.
    #[serde(default)]
    pub exclude: Option<Vec<String>>,
}

/// `PATCH /api/v1/repos/{name}` response: the repo's state after the patch.
#[derive(Serialize, Debug)]
pub struct PatchResponse {
    /// The (unchanged) label.
    pub repo: String,
    /// The current `repo_marker.root_path`.
    pub root_path: Option<String>,
}

/// `POST /api/v1/repos/{name}/archive` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ArchiveRequest {
    /// `true` archives (the default — the route accepts an empty body),
    /// `false` un-archives.
    #[serde(default = "default_archived")]
    pub archived: bool,
}

/// An archive request with no body archives.
fn default_archived() -> bool {
    true
}

impl Default for ArchiveRequest {
    /// The empty body: archive.
    fn default() -> Self {
        Self {
            archived: default_archived(),
        }
    }
}

/// `POST /api/v1/repos/{name}/archive` response.
#[derive(Serialize, Debug)]
pub struct ArchiveResponse {
    /// The label whose flag was flipped.
    pub repo: String,
    /// Its `repo_marker.archived` value after the write.
    pub archived: bool,
}

/// `DELETE /api/v1/repos/{name}` response — the per-table counters
/// [`repo_drop::drop_repo`] reports.
#[derive(Serialize, Debug)]
pub struct DisconnectResponse {
    /// The label that was disconnected.
    pub repo: String,
    /// `code_symbols` rows removed.
    pub symbols_removed: u64,
    /// `indexed_files` rows removed.
    pub files_removed: u64,
    /// `edges` rows removed (file nodes on either side).
    pub edges_removed: u64,
}

/// Register `req.root` under `req.repo` (default: the root's basename) by
/// upserting `repo_marker.root_path`.
///
/// A label already registered under a DIFFERENT root is a `400`: reusing it
/// would silently repoint an existing code index at a foreign checkout (the
/// same collision `cli::lazy_reindex` refuses to reindex through). Re-
/// connecting the SAME root is idempotent.
pub fn connect(ctx: &mut Ctx<'_>, req: ConnectRequest) -> Result<ConnectResponse> {
    let root = canonical_root(&req.root)?;
    let repo = match req.repo {
        Some(label) => label,
        None => basename(&root)?,
    };
    let conn = ctx.conn()?;
    if let Some(existing) = stored_root(conn, &repo)?
        && existing != root
    {
        return Err(Error::BadRequest(format!(
            "repo `{repo}` is already connected to `{existing}`; \
             disconnect it first or choose another label"
        )));
    }
    code_row::upsert_repo_root(conn, &repo, &root)?;
    Ok(ConnectResponse {
        repo,
        root_path: root,
        job_id: None,
    })
}

/// Apply a `PATCH` to `name`. `root` is the only modelled field; any other
/// one present is `501 unsupported`. An empty patch is a no-op read-back.
pub fn patch(ctx: &mut Ctx<'_>, name: &str, req: PatchRequest) -> Result<PatchResponse> {
    if req.name.is_some() || req.branch.is_some() || req.include.is_some() || req.exclude.is_some()
    {
        return Err(Error::Unsupported(
            "repo rename/branch/include/exclude are not modelled; \
             re-index under a new label instead"
                .into(),
        ));
    }
    let root = req.root.as_deref().map(canonical_root).transpose()?;
    let conn = ctx.conn()?;
    require_marker(conn, name)?;
    if let Some(root) = &root {
        code_row::upsert_repo_root(conn, name, root)?;
    }
    Ok(PatchResponse {
        repo: name.to_string(),
        root_path: stored_root(conn, name)?,
    })
}

/// Set `repo_marker.archived` for `name`. An archived repo refuses
/// `POST /api/v1/index/runs` and is skipped by lazy reindex; nothing is
/// deleted and its memories stay searchable. This — not [`disconnect`] —
/// is the "stop indexing, keep the memories" action.
pub fn archive(ctx: &mut Ctx<'_>, name: &str, req: ArchiveRequest) -> Result<ArchiveResponse> {
    let conn = ctx.conn()?;
    let updated = conn.execute(
        "UPDATE repo_marker SET archived = ?2 WHERE repo = ?1",
        rusqlite::params![name, i64::from(req.archived)],
    )?;
    if updated == 0 {
        return Err(not_found(name));
    }
    Ok(ArchiveResponse {
        repo: name.to_string(),
        archived: req.archived,
    })
}

/// Drop `name`'s whole code index (`store::repo_drop`), keeping its
/// memories. Unknown label → `404`.
///
/// Not a "stop indexing" action: the `repo_marker` row is dropped with the
/// index, so under `COMEMORY_INDEXING_AUTO_REINDEX=lazy` (the default) the
/// next `search-code` / `context` run from that checkout treats the repo
/// as never indexed and spawns a background `index-code` that rebuilds it.
/// Only `hook` / `off` leave a disconnected repo un-indexed; use
/// [`archive`] to keep the memories and stop indexing under every mode.
pub fn disconnect(ctx: &mut Ctx<'_>, name: &str) -> Result<DisconnectResponse> {
    let conn = ctx.conn()?;
    require_marker(conn, name)?;
    let counts = repo_drop::drop_repo(conn, name)?;
    Ok(DisconnectResponse {
        repo: name.to_string(),
        symbols_removed: counts.symbols_removed,
        files_removed: counts.files_removed,
        edges_removed: counts.edges_removed,
    })
}

/// Canonicalize `root` into the absolute string `repo_marker.root_path`
/// stores. A path that does not exist is a `400`, not a silent write of a
/// root nothing can be indexed from.
fn canonical_root(root: &str) -> Result<String> {
    let canonical = Path::new(root)
        .canonicalize()
        .map_err(|e| Error::BadRequest(format!("root `{root}` is unusable: {e}")))?;
    Ok(canonical.to_string_lossy().into_owned())
}

/// The default repo label: `root`'s last path component.
fn basename(root: &str) -> Result<String> {
    Path::new(root)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| Error::BadRequest(format!("cannot derive a repo label from `{root}`")))
}

/// `repo_marker.root_path` for `repo`, or `None` when there is no marker
/// row (or its root is NULL).
fn stored_root(conn: &Connection, repo: &str) -> Result<Option<String>> {
    let root: Option<Option<String>> = conn
        .query_row(
            "SELECT root_path FROM repo_marker WHERE repo = ?1",
            [repo],
            |r| r.get(0),
        )
        .optional()?;
    Ok(root.flatten())
}

/// `Ok(())` when a `repo_marker` row exists for `repo`, else [`not_found`].
fn require_marker(conn: &Connection, repo: &str) -> Result<()> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM repo_marker WHERE repo = ?1)",
        [repo],
        |r| r.get(0),
    )?;
    if exists { Ok(()) } else { Err(not_found(repo)) }
}

/// The shared `404` for an unknown repo label.
fn not_found(repo: &str) -> Error {
    Error::NotFound(format!("no connected repo named `{repo}`"))
}

#[cfg(test)]
#[path = "tests/repo_admin.rs"]
mod tests;
