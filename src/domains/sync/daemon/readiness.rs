//! The coordinator's owner-local readiness answer (the `status` op): who it
//! is, what it serves and what it is doing. It never carries the credential
//! secret, `COMEMORY_API_KEY` or the control token.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Paths;
use crate::domains::sync::drain::report::End;
use crate::domains::sync::replica::restore_state;
use crate::prelude::*;
use crate::store::readiness::{self, StoreReadiness};

/// Everything `status` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Readiness {
    /// Control protocol version.
    pub protocol: u32,
    /// The coordinator binary's version.
    pub version: String,
    /// The coordinator binary's canonical path.
    pub binary: PathBuf,
    /// `<dev>:<ino>` of the executable file it runs (#258); absent from a
    /// coordinator that predates it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_file: Option<String>,
    /// Its process id.
    pub pid: u32,
    /// Random per start: two starts of one pid never share it.
    pub instance: String,
    /// When it started (RFC 3339).
    pub started_at: String,
    /// The canonical data directory it serves.
    pub data_dir: PathBuf,
    /// The socket it answers on.
    pub socket: PathBuf,
    /// `launchd`, `systemd`, `process`, `external` or `foreground`.
    pub supervisor: String,
    /// The database, probed read-only.
    pub store: StoreState,
    /// The credential it would exchange under.
    pub auth: AuthView,
    /// Its passes and channel.
    pub sync: SyncView,
}

/// [`StoreReadiness`] plus an unverified restore and a probe that failed —
/// the store's health (#256, B-8), as [`probe_store`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreState {
    /// No database yet.
    Absent,
    /// At this build's schema.
    Ready,
    /// Behind; a writable command migrates it.
    MigrationPending,
    /// Behind, and this build's last long-lived open failed migrating it.
    MigrationFailed,
    /// Ahead; written by a newer build.
    TooNew,
    /// At this build's schema, but restored and its erasures not merged:
    /// it refuses to exchange until `comemory backup merge-erasures`.
    RestoreUnverified,
    /// The probe failed.
    Error,
}

impl StoreState {
    /// The serialized name, for a text line.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Ready => "ready",
            Self::MigrationPending => "migration_pending",
            Self::MigrationFailed => "migration_failed",
            Self::TooNew => "too_new",
            Self::RestoreUnverified => "restore_unverified",
            Self::Error => "error",
        }
    }

    /// Whether nothing is wrong: a current store, or none yet (the first
    /// write creates it). Every other state needs an operator.
    #[must_use]
    pub const fn is_healthy(self) -> bool {
        matches!(self, Self::Ready | Self::Absent)
    }
}

/// Probe `paths`' store read-only: [`crate::store::readiness::probe`], and a
/// current store whose restore is still unverified (`restore.pending` or
/// the `replica_restore_state` key) reads [`StoreState::RestoreUnverified`].
/// Creates, migrates and writes nothing.
#[must_use]
pub fn probe_store(paths: &Paths) -> StoreState {
    let db = paths.db_path();
    match readiness::probe(&db) {
        Ok(StoreReadiness::Ready) => match restore_unverified(paths, &db) {
            Ok(false) => StoreState::Ready,
            Ok(true) => StoreState::RestoreUnverified,
            Err(_) => StoreState::Error,
        },
        Ok(state) => state.into(),
        Err(_) => StoreState::Error,
    }
}

fn restore_unverified(paths: &Paths, db: &Path) -> Result<bool> {
    if restore_state::pending_path(paths).try_exists()? {
        return Ok(true);
    }
    Ok(restore_state::read(&readiness::open_probe(db)?)?.is_some())
}

impl From<StoreReadiness> for StoreState {
    fn from(state: StoreReadiness) -> Self {
        match state {
            StoreReadiness::Absent => Self::Absent,
            StoreReadiness::Ready => Self::Ready,
            StoreReadiness::MigrationPending => Self::MigrationPending,
            StoreReadiness::MigrationFailed => Self::MigrationFailed,
            StoreReadiness::TooNew => Self::TooNew,
        }
    }
}

/// The credential's public face.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthView {
    /// Which of the four states holds.
    pub state: AuthState,
    /// Platform base URL, when authenticated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    /// Organization slug, when authenticated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_slug: Option<String>,
    /// Workspace id, when authenticated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// The key's display prefix, when authenticated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_prefix: Option<String>,
}

/// Whether the coordinator may exchange.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    /// No credential: local work only.
    #[default]
    LoggedOut,
    /// A usable credential.
    Authenticated,
    /// A logout barrier stands until the next login.
    LoggedOutBarrier,
    /// `auth.json` exists but this build cannot use it.
    Unusable,
}

/// Passes and channel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncView {
    /// Idle, running a pass, or stopping.
    pub state: SyncState,
    /// The reconciliation interval in force.
    pub interval_secs: u64,
    /// Passes finished since start.
    pub passes: u64,
    /// Whether work is waiting for the next pass.
    pub queued: bool,
    /// The workspace channel.
    pub channel: ChannelState,
    /// The last finished pass.
    #[serde(default)]
    pub last_pass: Option<PassSummary>,
    /// The last successful verify (RFC 3339).
    #[serde(default)]
    pub last_verify_at: Option<String>,
}

/// What the pass worker is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    /// Waiting for work.
    #[default]
    Idle,
    /// A pass is running.
    Running,
    /// Shutting down.
    Stopping,
}

/// The workspace channel's state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelState {
    /// Not followed: no usable credential.
    #[default]
    Off,
    /// Minting a ticket or opening the socket.
    Connecting,
    /// Open.
    Connected,
    /// Waiting before the next attempt.
    Backoff,
}

/// Why a pass ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// The reconciliation interval.
    Tick,
    /// A hook-fired `sync --action auto`.
    Hook,
    /// A local write whose inline push did not finish.
    Save,
    /// A workspace-channel nudge.
    Channel,
    /// `watch --once` or another caller waiting for a catch-up.
    CatchUp,
    /// Credentials or config changed.
    Reload,
}

/// One finished pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassSummary {
    /// What triggered it (the first trigger of a coalesced set).
    pub trigger: Trigger,
    /// When it ended (RFC 3339).
    pub finished_at: String,
    /// Whether a usable credential was found.
    pub logged_in: bool,
    /// How its exchange ended, when it exchanged.
    #[serde(default)]
    pub end: Option<End>,
    /// Entries applied here.
    pub pulled: u32,
    /// Operations the upstream accepted.
    pub pushed: u32,
    /// Whether the store was ready; a skipped pass did nothing.
    pub store: StoreState,
    /// The first failure, if any.
    #[serde(default)]
    pub error: Option<String>,
}
