//! The coordinator's owner-local readiness answer (the `status` op): who it
//! is, what it serves and what it is doing. It never carries the credential
//! secret, `COMEMORY_API_KEY` or the control token.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domains::sync::drain::report::End;
use crate::store::readiness::StoreReadiness;

/// Everything `status` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Readiness {
    /// Control protocol version.
    pub protocol: u32,
    /// The coordinator binary's version.
    pub version: String,
    /// The coordinator binary's canonical path.
    pub binary: PathBuf,
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

/// [`StoreReadiness`] plus a probe that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreState {
    /// No database yet.
    Absent,
    /// At this build's schema.
    Ready,
    /// Behind; a writable command migrates it.
    MigrationPending,
    /// Ahead; written by a newer build.
    TooNew,
    /// The probe failed.
    Error,
}

impl From<StoreReadiness> for StoreState {
    fn from(state: StoreReadiness) -> Self {
        match state {
            StoreReadiness::Absent => Self::Absent,
            StoreReadiness::Ready => Self::Ready,
            StoreReadiness::MigrationPending => Self::MigrationPending,
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
