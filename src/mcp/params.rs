//! MCP-local parameter types.
//!
//! Most tools take the command core's own `Request`, which already derives
//! `Deserialize` + `JsonSchema`. `feedback` and the architecture tools need
//! adapter-owned shapes: feedback changes verdict provenance, while architecture
//! maps nested CLI arguments and the model body to domain cores.

use serde::Deserialize;
use serde_json::Value;

use crate::domains::architecture::model::{MAX_BYTES, Model};
use crate::domains::architecture::scaffold::Options;
use crate::domains::learning::feedback;
use crate::prelude::*;

/// Shared clustering parameters for architecture scaffold and check.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureShapeParams {
    /// Repo label, defaulting to the MCP session scope.
    #[serde(default)]
    pub repo: Option<String>,
    /// Directory-prefix depth used to cluster indexed files.
    #[serde(default)]
    pub depth: Option<usize>,
    /// Highest-ranked components retained in the scaffold.
    #[serde(default)]
    pub max_components: Option<usize>,
    /// Minimum projected edge weight retained in the scaffold.
    #[serde(default)]
    pub min_edge_weight: Option<i64>,
}

impl ArchitectureShapeParams {
    /// The domain options with absent MCP fields set to CLI-equivalent defaults.
    pub fn options(&self) -> Options {
        let defaults = Options::default();
        Options {
            depth: self.depth.unwrap_or(defaults.depth),
            max_components: self.max_components.unwrap_or(defaults.max_components),
            min_edge_weight: self.min_edge_weight.unwrap_or(defaults.min_edge_weight),
        }
    }
}

/// Parameters for saving an already-enriched architecture model.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureSaveParams {
    /// Repo label, defaulting to the MCP session scope.
    #[serde(default)]
    pub repo: Option<String>,
    /// Complete schema-1 model to validate and store, bounded before parsing.
    pub model: Value,
}

impl ArchitectureSaveParams {
    /// Bound the raw body before deserializing it as the domain model.
    pub fn parse_model(self) -> Result<Model> {
        let raw = serde_json::to_vec(&self.model)?;
        if raw.len() > MAX_BYTES {
            return Err(Error::Usage(format!(
                "architecture model is {} bytes; maximum is {MAX_BYTES}",
                raw.len()
            )));
        }
        Ok(serde_json::from_slice(&raw)?)
    }
}

/// Requested rendering for a stored architecture model.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ArchitectureShowFormat {
    /// Return the stored model JSON.
    Json,
    /// Return deterministic Mermaid flowchart source.
    Mermaid,
}

/// Parameters for reading a stored architecture model.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureShowParams {
    /// Repo label, defaulting to the MCP session scope.
    #[serde(default)]
    pub repo: Option<String>,
    /// Stored model JSON by default, or Mermaid source.
    pub format: Option<ArchitectureShowFormat>,
}

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
