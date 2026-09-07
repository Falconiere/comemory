//! `api::delete::{Response, run}` — the shared middle of `comemory delete` /
//! `DELETE /api/v1/memories/{id}`. Moved out of `cli::delete::run` (Binding
//! Rule 1). The confirm gate is transport-level (`?confirm=true`), not part
//! of this module — the CLI has no `--confirm` concept, so `run` takes a
//! plain `id`, not a `Request` struct.

use serde::Serialize;

use crate::api::Ctx;
use crate::prelude::*;
use crate::store::{memory_row, sync_log};
use time::OffsetDateTime;

/// `comemory delete` / `DELETE /api/v1/memories/{id}` response.
#[derive(Serialize, Debug)]
pub struct Response {
    /// Canonical id of the soft-deleted memory.
    pub deleted: String,
    /// The delete left the derived artifacts stale: `edge_fts` could not be
    /// rebuilt after the memory's edges went. The delete itself committed —
    /// this is a freshness warning, not a failure — but relation search is
    /// behind until the next write refreshes it. Omitted from the JSON when
    /// false, as in `gc`'s report.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub derived_stale: bool,
}

/// Soft-delete one memory: move the markdown file into `memories/.trash/`
/// and mirror the delete into `comemory.db`, via the shared
/// `cli::delete::soft_delete` helper (kept in `cli::delete` — it is also
/// reused by `comemory prune`'s low-value apply path).
pub fn run(ctx: &mut Ctx<'_>, id: &str) -> Result<Response> {
    let paths = ctx.paths;
    let conn = ctx.conn()?;
    let (deleted, content_hash, derived_stale) = crate::cli::delete::soft_delete(paths, conn, id)?;
    append_local_tombstone(conn, &deleted, &content_hash);
    Ok(Response {
        deleted,
        derived_stale,
    })
}

/// Best-effort sync-log row for a local delete — the delete itself already
/// committed, so a failure here is logged rather than propagated.
fn append_local_tombstone(conn: &mut rusqlite::Connection, memory_id: &str, content_hash: &str) {
    let at = match memory_row::iso_format(OffsetDateTime::now_utc()) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "sync_log tombstone skipped: timestamp format failed");
            return;
        }
    };
    let result = (|| -> Result<()> {
        let tx = conn.transaction()?;
        sync_log::append(
            &tx,
            sync_log::SyncOp::Tombstone,
            memory_id,
            content_hash,
            &at,
            sync_log::SyncOrigin::Local,
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(e) = result {
        tracing::warn!(
            memory_id,
            error = %e,
            "sync_log tombstone append failed after delete"
        );
    }
}
