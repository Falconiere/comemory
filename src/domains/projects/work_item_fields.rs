//! A work item as a plan operation carries it: its enums, its assignee, the
//! patch and the proposed create, from one field list in the platform's zod
//! key order (`project-plan-operations.ts`). The patch has no `id`; absent
//! and `null` differ as in [`super::operation_fields`].

use serde::{Deserialize, Serialize};

use crate::domains::projects::operations::{Nullable, non_null};
use crate::domains::projects::principal::PrincipalType;

/// A work item's kind.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemKind {
    /// A user-facing capability.
    Feature,
    /// A unit of work.
    Task,
    /// A defect.
    Bug,
    /// An investigation.
    Research,
}

/// A work item's priority.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPriority {
    /// Drop everything.
    Urgent,
    /// Next up.
    High,
    /// The default.
    Normal,
    /// When there is time.
    Low,
    /// Unranked.
    None,
}

/// Who a work item is assigned to: a human or a project agent's id.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Assignee {
    /// `user` or `project_agent`.
    pub principal_type: PrincipalType,
    /// The principal's id, at least one character.
    pub principal_id: String,
}

/// A work item's writable fields, every one optional.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkItemPatch {
    /// Parent item, or `null` for a top-level item.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_work_item_id: Option<Nullable<String>>,
    /// Milestone, or `null` for none.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub milestone_id: Option<Nullable<String>>,
    /// Its kind.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub kind: Option<WorkItemKind>,
    /// One-line title, 1–120.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub title: Option<String>,
    /// Free text, up to 8000.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub description: Option<String>,
    /// Its priority.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub priority: Option<WorkItemPriority>,
    /// Size estimate ≥ 0, or `null`.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub estimate: Option<Nullable<i64>>,
    /// Assignee, or `null` for none.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub assignee: Option<Nullable<Assignee>>,
    /// One of the project's repositories, or `null`.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub repo: Option<Nullable<String>>,
    /// Display order, ≥ 0.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub position: Option<i64>,
}

/// A new work item: the patch fields with a required kind and title, plus
/// its identity.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProposedWorkItem {
    /// Parent item, or `null` for a top-level item.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_work_item_id: Option<Nullable<String>>,
    /// Milestone, or `null` for none.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub milestone_id: Option<Nullable<String>>,
    /// Its kind.
    pub kind: WorkItemKind,
    /// One-line title, 1–120.
    pub title: String,
    /// Free text, up to 8000.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub description: Option<String>,
    /// Its priority.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub priority: Option<WorkItemPriority>,
    /// Size estimate ≥ 0, or `null`.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub estimate: Option<Nullable<i64>>,
    /// Assignee, or `null` for none.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub assignee: Option<Nullable<Assignee>>,
    /// One of the project's repositories, or `null`.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub repo: Option<Nullable<String>>,
    /// Display order, ≥ 0.
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub position: Option<i64>,
    /// Client-generated UUID.
    pub id: String,
}
