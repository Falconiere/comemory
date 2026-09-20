//! The one MCP-local parameter type.
//!
//! Every other tool takes the command core's own `Request`, which already
//! derives `Deserialize` + `JsonSchema`. `feedback` cannot: its core defaults
//! `source` to `explicit` (a typed verdict is a human one), while a verdict an
//! agent inferred is `implicit` and must never enter the golden harvest. So
//! the agent-facing shape drops `source` for a boolean the model can only set
//! truthfully, and this module does the mapping.

use serde::Deserialize;

use crate::domains::learning::feedback;

/// `feedback` tool parameters — `feedback::Request` with provenance replaced
/// by [`FeedbackParams::confirmed_by_user`].
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeedbackParams {
    /// Id of the originating recall (`q-<yyyymmdd>-<8hex>`), as returned by
    /// `find`, `search`, `search_code` or `context`.
    pub query_id: String,
    /// Memory ids that were used.
    #[serde(default)]
    pub used: Vec<String>,
    /// Memory ids that were judged irrelevant.
    #[serde(default)]
    pub irrelevant: Vec<String>,
    /// Code-symbol ids that were used.
    #[serde(default)]
    pub used_code: Vec<String>,
    /// Code-symbol ids that were judged irrelevant.
    #[serde(default)]
    pub irrelevant_code: Vec<String>,
    /// True only when the user stated the verdict. Stored as `manual`;
    /// everything else is `implicit` and never enters the golden harvest.
    #[serde(default)]
    pub confirmed_by_user: bool,
}

impl From<FeedbackParams> for feedback::Request {
    /// `confirmed_by_user` becomes the core's `source` word: `explicit` (which
    /// `feedback_tracking::Source` stores as `manual`) when the user stated
    /// the verdict, `implicit` otherwise. Never `None` — the core's default
    /// for an absent `source` is `explicit`, the wrong half for an agent.
    fn from(params: FeedbackParams) -> Self {
        let source = if params.confirmed_by_user {
            "explicit"
        } else {
            "implicit"
        };
        Self {
            query_id: params.query_id,
            used: params.used,
            irrelevant: params.irrelevant,
            used_code: params.used_code,
            irrelevant_code: params.irrelevant_code,
            source: Some(source.to_string()),
        }
    }
}

#[cfg(test)]
#[path = "tests/params.rs"]
mod tests;
