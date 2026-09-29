//! The entities a plan operation carries — patches and proposed creates —
//! for criteria, milestones and the charter, with the platform's field
//! lists and zod key order (`project-plan-operations.ts`); work items are
//! [`super::work_item_fields`]. A patch has no identity field, and every
//! type denies unknown keys, so an identity change is refused, not stripped.
//! A `.nullable().optional()` field is `Option<Nullable<T>>`: absent leaves
//! the stored value, `null` clears it; any other optional field refuses
//! `null`, as zod's `.optional()` does.

use serde::{Deserialize, Serialize};

use crate::domains::projects::operations::{Nullable, non_null};

/// A criterion's evidence policy.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRequirement {
    /// No evidence needed.
    None,
    /// Self-reported evidence suffices.
    Reported,
    /// Verified evidence is required.
    Verified,
}

/// A milestone's status.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MilestoneStatus {
    /// Not started.
    Planned,
    /// In progress.
    Active,
    /// Done.
    Completed,
    /// Abandoned.
    Canceled,
}

/// The charter fields a `project.update` may change.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProjectPatch {
    /// Display name, 1–120.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub name: Option<String>,
    /// The outcome, 1–2000.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub outcome: Option<String>,
    /// Up to 50 constraints, each 1–500.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub constraints: Option<Vec<String>>,
    /// Up to 50 non-goals, each 1–500.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub non_goals: Option<Vec<String>>,
    /// A date, or `null` to clear it.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub target_date: Option<Nullable<String>>,
    /// Up to 50 canonical `owner/name` repositories.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub repositories: Option<Vec<String>>,
}

/// A criterion's writable fields, every one optional.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CriterionPatch {
    /// Free text, 1–500.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub description: Option<String>,
    /// Whether completion needs it.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub required: Option<bool>,
    /// Its evidence policy.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub evidence_requirement: Option<EvidenceRequirement>,
    /// Display order, ≥ 0.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub position: Option<i64>,
}

/// A new criterion: the patch fields with a required description, its
/// identity, and the item it belongs to (absent or `null`: project-level).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProposedCriterion {
    /// Free text, 1–500.
    pub description: String,
    /// Whether completion needs it.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub required: Option<bool>,
    /// Its evidence policy.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub evidence_requirement: Option<EvidenceRequirement>,
    /// Display order, ≥ 0.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub position: Option<i64>,
    /// Client-generated UUID.
    pub id: String,
    /// Owning item; never patchable afterwards.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub work_item_id: Option<Nullable<String>>,
}

/// A milestone's writable fields, every one optional.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MilestonePatch {
    /// Display name, 1–120.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub name: Option<String>,
    /// Free text, up to 2000.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub description: Option<String>,
    /// `YYYY-MM-DD` or RFC 3339.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub target_date: Option<String>,
    /// Display order, ≥ 0.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub position: Option<i64>,
    /// Its status.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub status: Option<MilestoneStatus>,
}

/// A new milestone: the patch fields with a required name and target
/// date, plus its identity.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProposedMilestone {
    /// Display name, 1–120.
    pub name: String,
    /// Free text, up to 2000.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub description: Option<String>,
    /// `YYYY-MM-DD` or RFC 3339.
    pub target_date: String,
    /// Display order, ≥ 0.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub position: Option<i64>,
    /// Its status.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub status: Option<MilestoneStatus>,
    /// Client-generated UUID.
    pub id: String,
}
