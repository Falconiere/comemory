//! Manifest compare + repair for `comemory sync --action verify`.
//!
//! AC-9: when local and remote bucket digests diverge, reset cursors and
//! re-pull / re-push so a missed log write can self-heal, then re-compare.

use rusqlite::Connection;
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::api::sync::ManifestResponse;
use crate::api::{self, Ctx};
use crate::config::{Config, Paths};
use crate::prelude::*;
use crate::store::sync_state;
use crate::sync::AuthFile;
use crate::sync::client;
use crate::sync::{pull, push};

/// Buckets that differ between local and remote manifests.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VerifyReport {
    /// Number of bucket digests that differ (0 = in sync).
    pub differing_buckets: u32,
    /// Local head seq.
    pub local_head_seq: i64,
    /// Remote head seq.
    pub remote_head_seq: i64,
    /// Indices of buckets that differ (0..255).
    pub bucket_indices: Vec<u32>,
    /// True when a repair pass ran and manifests match afterwards.
    pub repaired: bool,
}

/// Compare local and remote manifests; when they differ, repair via full
/// pull+push reconcile and re-compare (AC-9).
pub fn verify_manifests(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    auth: &AuthFile,
    workspace_id: &str,
) -> Result<VerifyReport> {
    let first = compare_once(paths, cfg, conn, auth, workspace_id)?;
    if first.bucket_indices.is_empty() {
        return Ok(VerifyReport {
            repaired: false,
            ..first
        });
    }
    repair_reconcile(paths, cfg, conn, auth, workspace_id)?;
    let after = compare_once(paths, cfg, conn, auth, workspace_id)?;
    Ok(VerifyReport {
        repaired: after.bucket_indices.is_empty(),
        differing_buckets: after.differing_buckets,
        local_head_seq: after.local_head_seq,
        remote_head_seq: after.remote_head_seq,
        bucket_indices: after.bucket_indices,
    })
}

fn compare_once(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    auth: &AuthFile,
    workspace_id: &str,
) -> Result<VerifyReport> {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    let local = api::sync::manifest::run(&mut ctx)?;
    let secret = auth.effective_secret();
    let remote = client::fetch_manifest(&auth.api_url, &secret, workspace_id)?;
    let indices = diff_buckets(&local, &remote);
    Ok(VerifyReport {
        differing_buckets: indices.len() as u32,
        local_head_seq: local.head_seq,
        remote_head_seq: remote.head_seq,
        bucket_indices: indices,
        repaired: false,
    })
}

fn repair_reconcile(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    auth: &AuthFile,
    workspace_id: &str,
) -> Result<()> {
    let at = now_iso()?;
    sync_state::ensure(conn, workspace_id, &auth.api_url)?;
    sync_state::set_pulled(conn, workspace_id, 0, &at)?;
    sync_state::set_pushed(conn, workspace_id, 0, &at)?;
    let _ = pull::run_pull(paths, cfg, conn, auth, workspace_id, 2000)?;
    let _ = push::run_push(paths, cfg, conn, auth, workspace_id, None, 2000)?;
    Ok(())
}

fn diff_buckets(local: &ManifestResponse, remote: &ManifestResponse) -> Vec<u32> {
    let mut out = Vec::new();
    for (i, (a, b)) in local.buckets.iter().zip(remote.buckets.iter()).enumerate() {
        if a != b {
            out.push(i as u32);
        }
    }
    out
}

fn now_iso() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Iso8601::DEFAULT)
        .map_err(|e| Error::Other(format!("timestamp: {e}")))
}

#[cfg(test)]
#[path = "tests/verify.rs"]
mod tests;
