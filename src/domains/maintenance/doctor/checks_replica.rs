//! The `store health` and `replica identity` doctor checks (#256, B-4/B-8):
//! the store's health as `sync daemon status` reports it, and the stream
//! identity with the erasure manifest that backs it.

use super::checks::{Check, fail, ok, warn};
use crate::config::Paths;
use crate::domains::sync::daemon::readiness::{StoreState, probe_store};
use crate::domains::sync::replica::erasure_manifest;
use crate::domains::sync::replica::identity::{self, MANIFEST_FILE};
use crate::domains::sync::replica::restore_state;
use crate::prelude::*;
use crate::store::{Connection, store_health};

/// The store's health: `ok` only when nothing needs an operator.
pub(super) fn store_health(paths: &Paths, conn: Option<&Connection>) -> Result<Check> {
    let state = probe_store(paths);
    let detail = match state {
        StoreState::MigrationFailed => store_health::failed_migration(&paths.db_path())
            .map_or_else(
                || state.as_str().to_string(),
                |r| format!("migration failed at {}: {}", r.at, r.detail),
            ),
        StoreState::RestoreUnverified => {
            let stored = conn.map(restore_state::read).transpose()?.flatten();
            format!(
                "restore unverified ({}) — run `comemory backup merge-erasures <manifest>`, or \
                 rerun the pending `comemory backup restore`",
                stored.as_deref().unwrap_or("restore.pending")
            )
        }
        other => other.as_str().to_string(),
    };
    Ok(if state.is_healthy() {
        ok("store health", detail)
    } else {
        fail("store health", detail)
    })
}

/// The stream identity and its erasure manifest: `warn` when the manifest
/// is torn or short of what the identity counts.
pub(super) fn replica_identity(paths: &Paths) -> Result<Check> {
    let Some(identity) = identity::read(paths)? else {
        return Ok(ok(
            "replica identity",
            "none yet — written on the first replica read or drain",
        ));
    };
    let manifest = erasure_manifest::read(&identity::file(paths, MANIFEST_FILE))?;
    let (lines, established) = manifest.as_ref().map_or((0, identity.erasures == 0), |m| {
        (m.lines.len(), m.established(identity.erasures))
    });
    let detail = format!(
        "epoch {}, device {}, erasure manifest {lines} line(s) of {} counted",
        identity.epoch, identity.device_id, identity.erasures
    );
    Ok(if established {
        ok("replica identity", detail)
    } else {
        warn(
            "replica identity",
            format!(
                "{detail} — the manifest is torn or short; keep a copy of it before any restore"
            ),
        )
    })
}
