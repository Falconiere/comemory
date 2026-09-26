//! The control protocol's frames (`protocol: 1`): newline-delimited JSON over
//! the coordinator's Unix socket, authenticated both ways before any op.
//!
//! 1. client → `{"hello":1,"nonce":NC}`
//! 2. server → `{"hello":1,"nonce":NS,"proof":…}` (checked before anything else is sent)
//! 3. client → `{"proof":…,"op":OP}`
//! 4. server → `{"ok":true,"result":…}` / `{"ok":false,"error":…}`; a
//!    `subscribe` then streams one [`Event`] per line.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domains::sync::daemon::readiness::{ChannelState, PassSummary, Trigger};

/// The protocol version this build speaks.
pub const PROTOCOL: u32 = 1;

/// Largest frame either side reads; a longer line is a protocol error.
pub const MAX_FRAME: usize = 64 * 1024;

/// The first frame each side sends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    /// Protocol version.
    pub hello: u32,
    /// This side's fresh nonce.
    pub nonce: String,
    /// The server's proof over the client's nonce (server hello only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
}

/// The client's one request on a connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// The client's proof over the server's nonce.
    pub proof: String,
    /// What it asks for.
    pub op: Op,
}

/// What a client may ask.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// Readiness, from in-memory state only.
    Status {},
    /// Merge work into the pending set.
    Wake(Wake),
    /// Queue a pass and answer when one that started after this request ends
    /// without `more`.
    CatchUp {},
    /// Re-read credentials and config, restart or stop the channel, queue a
    /// catch-up.
    Reload {},
    /// Stream events until the client leaves.
    Subscribe {},
    /// Stop gracefully.
    Shutdown {},
}

/// A wake's payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wake {
    /// A checkout to index first (the hook's `--path`).
    #[serde(default)]
    pub checkout: Option<PathBuf>,
    /// Who woke the coordinator.
    pub reason: Trigger,
}

/// The server's answer to a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    /// Whether the op was accepted.
    pub ok: bool,
    /// The op's result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Why it was refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One streamed event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// A pass began.
    PassStarted {
        /// What triggered it.
        trigger: Trigger,
    },
    /// A pass ended.
    PassFinished(PassSummary),
    /// The workspace channel changed state.
    Channel {
        /// Its new state.
        state: ChannelState,
    },
}
