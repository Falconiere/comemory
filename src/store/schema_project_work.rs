//! Declared schema — the project work graph: work items, their `blocks`
//! dependencies, execution attempts and the work packets handed to them.
//! Every link pairs the row's own `project_id` with the id it names, against a
//! `(id, project_id)` unique index, so nothing points across projects. The
//! deviations from the platform are listed in [`super::schema_projects`].

use toolu_orm::core::column::{Integer, Text};
use toolu_orm::table;

#[table(name = "project_work_items")]
#[unique_index("project_work_items_project_number_uidx", project_id, number)]
#[unique_index("project_work_items_id_project_uidx", id, project_id)]
#[foreign_key(
    name = "project_work_items_milestone_project_fk",
    columns(milestone_id, project_id),
    references = "project_milestones(id, project_id)"
)]
#[foreign_key(
    name = "project_work_items_parent_project_fk",
    columns(parent_work_item_id, project_id),
    references = "project_work_items(id, project_id)"
)]
/// `project_work_items`: a numbered work item, nestable under a parent item
/// and a milestone of its own project. Both links are `NO ACTION`: a parent or
/// milestone still named cannot be deleted.
pub struct ProjectWorkItems {
    /// UUID minted by the command core.
    #[column(primary_key)]
    pub id: Text,
    /// Owning project; deleting it cascades here.
    #[column(not_null, references = "projects(id)", on_delete = "cascade")]
    pub project_id: Text,
    /// Unique per project.
    #[column(not_null)]
    pub number: Integer,
    /// Parent item in the same project, if nested.
    pub parent_work_item_id: Text,
    /// Milestone in the same project, if any.
    pub milestone_id: Text,
    /// Item kind (`task`, `bug`, …).
    #[column(not_null)]
    pub kind: Text,
    /// One-line title.
    #[column(not_null)]
    pub title: Text,
    /// Free-text description.
    #[column(not_null)]
    pub description: Text,
    /// Item status (`backlog`, …).
    #[column(not_null, default = "'backlog'")]
    pub status: Text,
    /// Priority (`normal`, …).
    #[column(not_null, default = "'normal'")]
    pub priority: Text,
    /// Size estimate, if given.
    pub estimate: Integer,
    /// Principal kind of the assignee.
    pub assignee_principal_type: Text,
    /// Principal id of the assignee.
    pub assignee_principal_id: Text,
    /// Canonical `owner/name`, when the item's evidence is scoped to one repo.
    pub repo: Text,
    /// Optimistic-concurrency counter.
    #[column(not_null, default = "1")]
    pub version: Integer,
    /// Display order among its siblings.
    #[column(not_null, default = "0")]
    pub position: Integer,
    /// Epoch milliseconds it was archived; `NULL` while active.
    pub archived_at: Integer,
    /// Epoch milliseconds the row was written.
    #[column(not_null, default = "unixepoch() * 1000")]
    pub created_at: Integer,
    /// Epoch milliseconds of the last change.
    #[column(not_null, default = "unixepoch() * 1000")]
    pub updated_at: Integer,
}

#[table(name = "project_work_item_dependencies")]
#[primary_key(project_id, blocker_id, blocked_id)]
#[foreign_key(
    name = "project_dependencies_blocker_project_fk",
    columns(blocker_id, project_id),
    references = "project_work_items(id, project_id)",
    on_delete = "cascade"
)]
#[foreign_key(
    name = "project_dependencies_blocked_project_fk",
    columns(blocked_id, project_id),
    references = "project_work_items(id, project_id)",
    on_delete = "cascade"
)]
/// `project_work_item_dependencies`: `blocker_id` blocks `blocked_id`, both in
/// this row's project; an item never blocks itself.
pub struct ProjectWorkItemDependencies {
    /// Owning project; deleting it cascades here.
    #[column(not_null, references = "projects(id)", on_delete = "cascade")]
    pub project_id: Text,
    /// Item that blocks.
    #[column(not_null)]
    pub blocker_id: Text,
    /// Item that is blocked; never the blocker itself.
    #[column(not_null, check = "blocker_id != blocked_id")]
    pub blocked_id: Text,
}

#[table(name = "project_executions")]
#[unique_index("project_executions_id_project_uidx", id, project_id)]
#[foreign_key(
    name = "project_executions_work_item_project_fk",
    columns(work_item_id, project_id),
    references = "project_work_items(id, project_id)",
    on_delete = "cascade"
)]
/// `project_executions`: one attempt at a work item. A resumed attempt links
/// the stale one to its successor through `superseded_by_execution_id`
/// (`NO ACTION`).
pub struct ProjectExecutions {
    /// UUID minted by the command core.
    #[column(primary_key)]
    pub id: Text,
    /// Owning project; deleting it cascades here.
    #[column(not_null, references = "projects(id)", on_delete = "cascade")]
    pub project_id: Text,
    /// Item attempted, in the same project.
    #[column(not_null)]
    pub work_item_id: Text,
    /// Principal kind of the actor.
    #[column(not_null)]
    pub actor_principal_type: Text,
    /// Principal id of the actor.
    #[column(not_null)]
    pub actor_principal_id: Text,
    /// Stored state (`active`, …); `stale` is derived on read.
    #[column(not_null, default = "'active'")]
    pub state: Text,
    /// Epoch milliseconds the attempt started.
    #[column(not_null, default = "unixepoch() * 1000")]
    pub started_at: Integer,
    /// Epoch milliseconds of the last heartbeat.
    pub heartbeat_at: Integer,
    /// Epoch milliseconds it finished.
    pub finished_at: Integer,
    /// Closing summary.
    pub result_summary: Text,
    /// Optimistic-concurrency counter.
    #[column(not_null, default = "1")]
    pub version: Integer,
    /// The attempt that replaced this stale one, if any.
    #[column(references = "project_executions(id)")]
    pub superseded_by_execution_id: Text,
}

#[table(name = "project_work_packets")]
#[foreign_key(
    name = "project_work_packets_execution_project_fk",
    columns(execution_id, project_id),
    references = "project_executions(id, project_id)",
    on_delete = "cascade"
)]
#[foreign_key(
    name = "project_work_packets_work_item_project_fk",
    columns(work_item_id, project_id),
    references = "project_work_items(id, project_id)",
    on_delete = "cascade"
)]
/// `project_work_packets`: the provenance of one work-packet response —
/// citation ids and content hashes only, never a copied body.
pub struct ProjectWorkPackets {
    /// UUID minted by the command core.
    #[column(primary_key)]
    pub id: Text,
    /// Owning project; deleting it cascades here.
    #[column(not_null, references = "projects(id)", on_delete = "cascade")]
    pub project_id: Text,
    /// Execution the packet was built for, in the same project.
    #[column(not_null)]
    pub execution_id: Text,
    /// Item the packet describes, in the same project.
    #[column(not_null)]
    pub work_item_id: Text,
    /// Plan version the packet was built against.
    #[column(not_null)]
    pub plan_version: Integer,
    /// Item version the packet was built against.
    #[column(not_null)]
    pub work_item_version: Integer,
    /// Retrieval query id the citations came from.
    #[column(not_null)]
    pub engine_query_id: Text,
    /// JSON `{ queryId, items: [{ type, id, contentHash, source }] }`.
    #[column(not_null, default = "'[]'")]
    pub citations: Text,
    /// Principal kind of the requester.
    #[column(not_null)]
    pub requester_principal_type: Text,
    /// Principal id of the requester.
    #[column(not_null)]
    pub requester_principal_id: Text,
    /// Epoch milliseconds the row was written.
    #[column(not_null, default = "unixepoch() * 1000")]
    pub created_at: Integer,
}
