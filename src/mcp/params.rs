//! MCP-local parameter types.
//!
//! Most tools take the command core's own `Request`, which already derives
//! `Deserialize` + `JsonSchema`. `feedback` and the architecture tools need
//! adapter-owned shapes: feedback changes verdict provenance, architecture
//! maps nested CLI arguments and the model body to domain cores, and
//! `project_show` picks the charter, the plan or the activity core by `view`.

use serde::Deserialize;
use serde_json::Value;

use crate::domains::architecture::model::{MAX_BYTES, Model};
use crate::domains::architecture::scaffold::Options;
use crate::domains::learning::feedback;
use crate::prelude::*;
use crate::utilities::project_error::ProjectError;

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
    /// Complete schema-1 model, bounded before domain-schema deserialization.
    pub model: Value,
}

impl ArchitectureSaveParams {
    /// Bound the serialized model before domain-schema deserialization.
    ///
    /// The MCP transport has already decoded the JSON value using serde_json's
    /// default nesting guard. This is the model-size limit, not a framing limit.
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

/// Which read of one project `project_show` returns.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProjectShowView {
    /// The charter, as `project show` reads it.
    #[default]
    Charter,
    /// The committed plan, as `project plan show` reads it.
    Plan,
    /// One page of the activity log, as `project activity` reads it.
    Activity,
    /// One page of evidence, as `project evidence list` reads it.
    Evidence,
}

/// `project_show` tool parameters: the `project show` / `project plan show` /
/// `project activity` / `project evidence list` request plus the view
/// choosing between them. `limit` and `cursor` belong to the two paged views,
/// `order` to `activity`, and `kind`, `trust` and `workItemId` to
/// `evidence`; any other view refuses them.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectShowParams {
    /// The project's UUID.
    pub id: String,
    /// `charter` (default), `plan`, `activity` or `evidence`.
    #[serde(default)]
    pub view: ProjectShowView,
    /// Page size: activity 1–200 (default 50), evidence 1–100 (default 50).
    #[serde(default)]
    pub limit: Option<i64>,
    /// The previous page's `nextCursor`.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Activity: `desc` (newest first, the default) or `asc` (oldest first).
    #[serde(default)]
    pub order: Option<String>,
    /// Evidence: only this kind.
    #[serde(default)]
    pub kind: Option<String>,
    /// Evidence: only this trust (`verified`, `self_reported`, `pending`,
    /// `invalid`).
    #[serde(default)]
    pub trust: Option<String>,
    /// Evidence: only evidence on this work item.
    #[serde(default, rename = "workItemId")]
    pub work_item_id: Option<String>,
}

impl ProjectShowParams {
    /// Refuse a field the chosen view does not read, naming the first one
    /// set, so a caller never believes it paged a charter or filtered an
    /// activity page: the schema-edge `400 invalid_request` `<field> is
    /// page_only` (`limit`, `cursor`), `activity_only` (`order`) or
    /// `evidence_only` (`kind`, `trust`, `workItemId`).
    pub fn page_fields_fit_the_view(&self) -> Result<()> {
        use ProjectShowView::{Activity, Evidence};
        let paged: &[ProjectShowView] = &[Activity, Evidence];
        let fields: [(&str, bool, &[ProjectShowView], &str); 6] = [
            ("limit", self.limit.is_some(), paged, "page_only"),
            ("cursor", self.cursor.is_some(), paged, "page_only"),
            ("order", self.order.is_some(), &[Activity], "activity_only"),
            ("kind", self.kind.is_some(), &[Evidence], "evidence_only"),
            ("trust", self.trust.is_some(), &[Evidence], "evidence_only"),
            (
                "workItemId",
                self.work_item_id.is_some(),
                &[Evidence],
                "evidence_only",
            ),
        ];
        let misplaced = fields
            .iter()
            .find(|(_, set, views, _)| *set && !views.contains(&self.view));
        match misplaced {
            Some((field, _, _, reason)) => Err(ProjectError::invalid_field(field, reason).into()),
            None => Ok(()),
        }
    }
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
