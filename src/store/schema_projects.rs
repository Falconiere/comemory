//! Declared schema — the engine's fourteen project tables (issue 325), ported
//! column for column from the platform's Drizzle `project-schema/*-table.ts`.
//! Charter tables and the registry live here; the work graph in
//! [`super::schema_project_work`]; proposals, evidence, activity and receipts in
//! [`super::schema_project_record`]. Composite keys, cascades and `NO ACTION`
//! links are the platform's; enum columns stay unconstrained `TEXT`, so a value
//! this build does not know stays readable.
//!
//! # Deviations from the platform schema
//!
//! - **No `workspace_id`:** one data directory is one workspace, so `slug`,
//!   `key_prefix` and a receipt's `(principal_type, principal_id,
//!   idempotency_key)` are unique on their own.
//! - **No user foreign keys:** `lead_user_id`, `created_by`, `resolved_by` and
//!   `reviewer_id` become `lead_`, `creator_`, `resolver_` and `reviewer_`
//!   `*_principal_type` / `*_principal_id` pairs — the engine has no `user`
//!   table, and role nouns match the platform's `principalColumns(role)`.
//! - **No `project_agent_grants`:** a grant is the capability envelope the
//!   platform stamps (issue 263); the engine never stores one.
//! - **No id default:** the platform mints UUIDs in the application, as the
//!   engine's command core will.
//! - **SQLite storage:** timestamps are `INTEGER` epoch milliseconds defaulting
//!   to `unixepoch() * 1000`, booleans `INTEGER` 0/1, JSON `TEXT`.

use toolu_orm::core::column::{Integer, Text};
use toolu_orm::core::table::{TableDef, TableSchema};
use toolu_orm::table;

use super::schema_project_record::{
    ProjectActivityEvents, ProjectApprovals, ProjectCommandReceipts, ProjectEvidence,
    ProjectEvidenceCriteria, ProjectPlanProposals,
};
use super::schema_project_work::{
    ProjectExecutions, ProjectWorkItemDependencies, ProjectWorkItems, ProjectWorkPackets,
};

/// Every project-owned table, leaf first: a table precedes every table its
/// foreign keys point at, so deleting in this order never trips a key, and
/// copying in reverse inserts parents before children. The rebuild copy walks
/// it today; hard deletion (#320) and transfer (#342) will walk it rather
/// than keep their own.
pub const PROJECT_TABLES: &[&str] = &[
    "project_evidence_criteria",
    "project_evidence",
    "project_work_packets",
    "project_executions",
    "project_criteria",
    "project_work_item_dependencies",
    "project_work_items",
    "project_milestones",
    "project_approvals",
    "project_plan_proposals",
    "project_activity_events",
    "project_command_receipts",
    "project_repositories",
    "projects",
];

/// The declarations behind [`PROJECT_TABLES`], in the same order.
#[must_use]
pub fn table_defs() -> Vec<TableDef> {
    vec![
        ProjectEvidenceCriteria::table_def(),
        ProjectEvidence::table_def(),
        ProjectWorkPackets::table_def(),
        ProjectExecutions::table_def(),
        ProjectCriteria::table_def(),
        ProjectWorkItemDependencies::table_def(),
        ProjectWorkItems::table_def(),
        ProjectMilestones::table_def(),
        ProjectApprovals::table_def(),
        ProjectPlanProposals::table_def(),
        ProjectActivityEvents::table_def(),
        ProjectCommandReceipts::table_def(),
        ProjectRepositories::table_def(),
        Projects::table_def(),
    ]
}

/// `projects`: one finite outcome — its charter, status, health and the plan
/// version it is on. `version` is the row's own optimistic-concurrency
/// counter; `current_plan_version` starts at `0` until a proposal is approved.
#[table(name = "projects")]
#[unique_index("projects_slug_uidx", slug)]
#[unique_index("projects_key_prefix_uidx", key_prefix)]
pub struct Projects {
    /// UUID minted by the command core.
    #[column(primary_key)]
    pub id: Text,
    /// URL slug, unique in the data directory.
    #[column(not_null)]
    pub slug: Text,
    /// Work-item key prefix, unique in the data directory.
    #[column(not_null)]
    pub key_prefix: Text,
    /// Display name.
    #[column(not_null)]
    pub name: Text,
    /// The finite outcome the project exists to reach.
    #[column(not_null)]
    pub outcome: Text,
    /// JSON `string[]`.
    #[column(not_null, default = "'[]'")]
    pub constraints: Text,
    /// JSON `string[]`.
    #[column(not_null, default = "'[]'")]
    pub non_goals: Text,
    /// Lifecycle status (`draft`, `active`, `paused`, …).
    #[column(not_null, default = "'draft'")]
    pub status: Text,
    /// Reported health (`unknown`, `on_track`, …).
    #[column(not_null, default = "'unknown'")]
    pub health: Text,
    /// Replaces the platform's `lead_user_id`.
    #[column(not_null)]
    pub lead_principal_type: Text,
    /// Principal id of the lead.
    #[column(not_null)]
    pub lead_principal_id: Text,
    /// Epoch milliseconds; `NULL` when the project has no deadline.
    pub target_date: Integer,
    /// Approved plan version; `0` until the first approval.
    #[column(not_null, default = "0")]
    pub current_plan_version: Integer,
    /// Optimistic-concurrency counter for the row.
    #[column(not_null, default = "1")]
    pub version: Integer,
    /// How the project completes (`manual`, …).
    #[column(not_null, default = "'manual'")]
    pub completion_policy: Text,
    /// Replaces the platform's `created_by`.
    #[column(not_null)]
    pub creator_principal_type: Text,
    /// Principal id of the creator.
    #[column(not_null)]
    pub creator_principal_id: Text,
    /// Epoch milliseconds the row was written.
    #[column(not_null, default = "unixepoch() * 1000")]
    pub created_at: Integer,
    /// Epoch milliseconds of the last change.
    #[column(not_null, default = "unixepoch() * 1000")]
    pub updated_at: Integer,
    /// Epoch milliseconds it was archived; `NULL` while active.
    pub archived_at: Integer,
}

/// `project_repositories`: a canonical `owner/name` repository the project
/// may use (at most 50 per project, enforced by the domain).
#[table(name = "project_repositories")]
#[primary_key(project_id, repo)]
pub struct ProjectRepositories {
    /// Owning project; deleting it cascades here.
    #[column(not_null, references = "projects(id)", on_delete = "cascade")]
    pub project_id: Text,
    /// Canonical `owner/name`.
    #[column(not_null)]
    pub repo: Text,
    /// Replaces the platform's `created_by`.
    #[column(not_null)]
    pub creator_principal_type: Text,
    /// Principal id of the creator.
    #[column(not_null)]
    pub creator_principal_id: Text,
    /// Epoch milliseconds the row was written.
    #[column(not_null, default = "unixepoch() * 1000")]
    pub created_at: Integer,
}

/// `project_milestones`: a dated checkpoint. `(id, project_id)` is the
/// composite key target a work item's `milestone_id` references.
#[table(name = "project_milestones")]
#[unique_index("project_milestones_id_project_uidx", id, project_id)]
pub struct ProjectMilestones {
    /// UUID minted by the command core.
    #[column(primary_key)]
    pub id: Text,
    /// Owning project; deleting it cascades here.
    #[column(not_null, references = "projects(id)", on_delete = "cascade")]
    pub project_id: Text,
    /// Display name.
    #[column(not_null)]
    pub name: Text,
    /// Free-text description.
    #[column(not_null)]
    pub description: Text,
    /// Epoch milliseconds.
    #[column(not_null)]
    pub target_date: Integer,
    /// Display order among its siblings.
    #[column(not_null, default = "0")]
    pub position: Integer,
    /// Milestone status (`planned`, …).
    #[column(not_null, default = "'planned'")]
    pub status: Text,
    /// Epoch milliseconds it was archived; `NULL` while active.
    pub archived_at: Integer,
}

#[table(name = "project_criteria")]
#[foreign_key(
    name = "project_criteria_work_item_project_fk",
    columns(work_item_id, project_id),
    references = "project_work_items(id, project_id)",
    on_delete = "cascade"
)]
/// `project_criteria`: a success criterion — project-level when
/// `work_item_id` is `NULL` (which skips the composite key), item-level and
/// held inside the project otherwise.
pub struct ProjectCriteria {
    /// UUID minted by the command core.
    #[column(primary_key)]
    pub id: Text,
    /// Owning project; deleting it cascades here.
    #[column(not_null, references = "projects(id)", on_delete = "cascade")]
    pub project_id: Text,
    /// Item the criterion belongs to; `NULL` for a project-level one.
    pub work_item_id: Text,
    /// Free-text description.
    #[column(not_null)]
    pub description: Text,
    /// Boolean 0/1.
    #[column(not_null, default = "1")]
    pub required: Integer,
    /// Evidence needed to accept it (`reported`, …).
    #[column(not_null, default = "'reported'")]
    pub evidence_requirement: Text,
    /// `open`, `accepted` or `waived`.
    #[column(not_null, default = "'open'")]
    pub resolution: Text,
    /// Human rationale recorded with a waiver.
    pub resolution_rationale: Text,
    /// Replaces the platform's `resolved_by`; `NULL` while unresolved.
    pub resolver_principal_type: Text,
    /// Principal id of the resolver.
    pub resolver_principal_id: Text,
    /// Display order among its siblings.
    #[column(not_null, default = "0")]
    pub position: Integer,
    /// Epoch milliseconds it was archived; `NULL` while active.
    pub archived_at: Integer,
}

/// `project_changes`: the body-free change feed (#324) — one id-only row per
/// committed project mutation, written in the mutation's own transaction, and
/// one `deleted` row per hard deletion. Deliberately **not** in
/// [`PROJECT_TABLES`] and without a foreign key to `projects`: a deletion row
/// must outlive the project it names. The `AUTOINCREMENT` key is the reader's
/// cursor, monotonic and never reused; nothing prunes the feed.
#[table(name = "project_changes")]
pub struct ProjectChanges {
    /// Monotonic position, and the cursor a reader resumes from.
    #[column(primary_key, autoincrement)]
    pub seq: Integer,
    /// The project that changed.
    #[column(not_null)]
    pub project_id: Text,
    /// The activity event recorded with the change; the project's own id for
    /// a deletion, whose audit trail is gone.
    #[column(not_null)]
    pub event_id: Text,
    /// Kind of entity the event happened to (`project`, …).
    #[column(not_null)]
    pub entity_type: Text,
    /// `changed` or `deleted`.
    #[column(not_null)]
    pub op: Text,
    /// Epoch milliseconds, the mutation's own timestamp.
    #[column(not_null)]
    pub created_at: Integer,
}

#[cfg(test)]
#[path = "tests/schema_projects.rs"]
mod tests;
