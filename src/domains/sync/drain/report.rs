//! What a pass (or a run of passes) did — the `exchange` leg every sync
//! report carries.

use serde::{Deserialize, Serialize};

/// Why a pass ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    /// Cursor at the captured head and nothing eligible left to send.
    #[default]
    CaughtUp,
    /// The pass budget ran out before the backlog did.
    Budget,
    /// The pull stalled and nothing eligible was left to send.
    Stalled,
    /// A full iteration moved nothing.
    NoProgress,
    /// A request failed after its in-pass retries, or the pass made none.
    Network,
    /// A logout or a coordinator shutdown stopped the run at a boundary.
    Cancelled,
}

/// The `exchange` leg of a sync report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    /// `legacy` or `replica-v1`, once negotiated.
    pub protocol: Option<String>,
    /// `full` or `partial`, once negotiated.
    pub coverage: Option<String>,
    /// Operations the upstream accepted.
    pub pushed: u32,
    /// Entries applied here.
    pub pulled: u32,
    /// Positions or operations held with a reason.
    pub held: u32,
    /// This client's operations settled by content or by the pulled feed.
    pub settled: u32,
    /// Operations the upstream refused for good.
    pub rejected: u32,
    /// Why the last pass ended.
    pub end: End,
    /// Whether another pass should run at once.
    pub more: bool,
    /// Passes run.
    pub passes: u32,
    /// Whether a pass replaced its stream view.
    pub rebootstrapped: bool,
    /// `ok`, `backoff`, `auth_suspended` or `protocol_error`.
    pub network: String,
    /// The pass met a policy change and recorded nothing; the drain reloads
    /// the policy and runs once more.
    #[serde(skip)]
    pub policy_changed: bool,
}

impl Report {
    /// Fold one pass into a run.
    pub fn absorb(&mut self, pass: Self) {
        self.protocol = pass.protocol.or(self.protocol.take());
        self.coverage = pass.coverage.or(self.coverage.take());
        self.pushed += pass.pushed;
        self.pulled += pass.pulled;
        self.held = pass.held;
        self.settled += pass.settled;
        self.rejected += pass.rejected;
        self.end = pass.end;
        self.more = pass.more;
        self.passes += 1;
        self.rebootstrapped |= pass.rebootstrapped;
        self.network = pass.network;
        self.policy_changed = pass.policy_changed;
    }
}
