//! `api::sources::{Request, run}` — the shared middle of `comemory
//! sources` / `GET /api/v1/sources`: optionally reconcile the SQLite
//! `source_roots` mirror against `sources.toml`, then list every
//! registered source with its per-status file counts. Moved out of
//! `cli::sources::run` (Binding Rule 1).
//!
//! `reconcile` is split out of the CLI's always-on behavior so a
//! `--read-only` HTTP server can list without the mirror-write side effect
//! (§Security "Read-only side-effect degradation"): the CLI always passes
//! `true`; the HTTP route passes `false` on a `--read-only` server.

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::prelude::*;
use crate::source::mirror;
use crate::source::registry::Registry;
use crate::store::sources;

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
}

/// List every registered source, reconciling the SQLite mirror against
/// `sources.toml` first when `req.reconcile` is set — so the report
/// reflects the durable source of truth even when the mirror fell behind
/// (e.g. a hand-edited `sources.toml`, or a run of `comemory
/// index`/`unindex` from another process).
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Vec<Row>> {
    let registry = Registry::new(ctx.paths.clone());
    let conn: &rusqlite::Connection = ctx.conn()?;
    if req.reconcile {
        mirror::reconcile(conn, &registry.load()?)?;
    }
    let mut rows = Vec::new();
    for root in sources::list(conn)? {
        let counts = sources::file_status_counts(conn, &root.id)?;
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
        });
    }
    Ok(rows)
}
