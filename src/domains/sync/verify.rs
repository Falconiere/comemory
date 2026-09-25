//! Manifest compare + repair for `comemory sync --action verify`.
//!
//! A `replica-v1` key compares per kind and repairs by a kind-scoped replay
//! ([`crate::domains::sync::drain::verify`]). A legacy key keeps today's
//! remedy (AC-9): when local and remote bucket digests diverge, reset both
//! legacy cursors and drain again — now through the drain loop rather than a
//! 2,000-entry cap — then re-compare. That repair re-imports everything, so
//! it can re-patch a memory edited locally since: a known legacy hazard.
//!
//! The caller holds the sync pass lock.

use crate::config::{Config, Paths};
use crate::domains::sync::AuthFile;
use crate::domains::sync::drain::negotiate::Protocol;
use crate::domains::sync::drain::session::{self, Legs, Mode, Opened, Session};
use crate::domains::sync::drain::verify::{self as replica_verify, Reopen, ReplicaVerify};
use crate::domains::sync::drain::{self, network};
use crate::domains::sync::exchange::ManifestResponse;
use crate::prelude::*;
use crate::store::{Connection, sync_log, sync_state};

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

/// What a verify reports, by the protocol the key speaks.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(untagged)]
pub enum Verified {
    /// A legacy key's one manifest.
    Legacy(VerifyReport),
    /// A `replica-v1` key's per-kind rows.
    Replica(ReplicaVerify),
}

/// Compare local and remote manifests; when they differ, repair and
/// re-compare.
///
/// # Errors
/// [`Error::Unavailable`] when the upstream cannot be reached (the key's
/// network state says why); store and configuration failures.
pub fn verify(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    auth: &AuthFile,
) -> Result<Verified> {
    let mut session = open(conn, cfg, auth)?;
    if session.protocol == Protocol::Replica {
        let reopen_session = |conn: &mut Connection| {
            open(conn, cfg, auth).map(|s| (s.protocol == Protocol::Replica).then_some(s))
        };
        let reopen = Reopen {
            paths,
            cfg,
            open: &reopen_session,
        };
        return replica_verify::run(conn, &reopen, &mut session).map(Verified::Replica);
    }
    let first = compare_once(conn, &session)?;
    if first.bucket_indices.is_empty() {
        return Ok(Verified::Legacy(first));
    }
    repair_reconcile(paths, cfg, conn, auth)?;
    let reopened = open(conn, cfg, auth)?;
    let after = compare_once(conn, &reopened)?;
    Ok(Verified::Legacy(VerifyReport {
        repaired: after.bucket_indices.is_empty(),
        ..after
    }))
}

/// A session for the key, or why there is none.
fn open(conn: &mut Connection, cfg: &Config, auth: &AuthFile) -> Result<Box<Session>> {
    match session::open(conn, cfg, auth, Mode::Manual)? {
        Opened::Ready(session) => Ok(session),
        Opened::Skipped(row) => Err(Error::Unavailable(format!(
            "verify: {} ({})",
            row.network_state,
            row.last_error.unwrap_or_default()
        ))),
    }
}

fn compare_once(conn: &Connection, session: &Session) -> Result<VerifyReport> {
    let local = crate::domains::sync::exchange::manifest::from_hashes(
        sync_log::head_seq(conn)?,
        session.policy.authorized_content_hashes(conn)?,
    );
    let remote = session
        .legacy
        .retrying(|t| t.get::<ManifestResponse>("/v1/sync/manifest", &[]))
        .map_err(|failure| Error::Unavailable(format!("verify: fetch manifest: {failure:?}")))?;
    let indices = diff_buckets(&local, &remote);
    Ok(VerifyReport {
        differing_buckets: u32::try_from(indices.len()).unwrap_or(u32::MAX),
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
) -> Result<()> {
    let at = network::now()?;
    let workspace_id = auth.workspace_id.as_str();
    sync_state::ensure(conn, workspace_id, &auth.api_url)?;
    sync_state::set_pulled(conn, workspace_id, 0, &at)?;
    sync_state::set_pushed(conn, workspace_id, 0, &at)?;
    let drained = drain::drain(paths, cfg, conn, auth, (Mode::Manual, Legs::Both))?;
    match drained.error {
        Some(error) => Err(Error::Unavailable(format!("verify repair: {error}"))),
        None => Ok(()),
    }
}

fn diff_buckets(local: &ManifestResponse, remote: &ManifestResponse) -> Vec<u32> {
    local
        .buckets
        .iter()
        .zip(&remote.buckets)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .filter_map(|(i, _)| u32::try_from(i).ok())
        .collect()
}

#[cfg(test)]
#[path = "tests/verify.rs"]
mod tests;
