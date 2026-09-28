//! Declared schema — what a project records about itself: plan proposals and
//! their one review decision, typed evidence and the criteria it may satisfy,
//! the append-only activity feed, and idempotent command receipts. The
//! deviations from the platform are listed in [`super::schema_projects`].

use toolu_orm::core::column::{Integer, Text};
use toolu_orm::table;

/// `project_plan_proposals`: an immutable plan-operation proposal — a change
/// is a new row, never an edit of a reviewed one.
#[table(name = "project_plan_proposals")]
pub struct ProjectPlanProposals {
    /// UUID minted by the command core.
    #[column(primary_key)]
    pub id: Text,
    /// Owning project; deleting it cascades here.
    #[column(not_null, references = "projects(id)", on_delete = "cascade")]
    pub project_id: Text,
    /// Plan version the operations apply to.
    #[column(not_null)]
    pub base_plan_version: Integer,
    /// `pending`, `approved`, `rejected`, …
    #[column(not_null, default = "'pending'")]
    pub state: Text,
    /// JSON `ProjectPlanOperation[]`.
    #[column(not_null)]
    pub operations: Text,
    /// JSON `string[]`.
    #[column(not_null, default = "'[]'")]
    pub assumptions: Text,
    /// JSON `string[]`.
    #[column(not_null, default = "'[]'")]
    pub risks: Text,
    /// Why the change is proposed.
    #[column(not_null)]
    pub rationale: Text,
    /// Principal kind of the proposer.
    #[column(not_null)]
    pub proposer_principal_type: Text,
    /// Principal id of the proposer.
    #[column(not_null)]
    pub proposer_principal_id: Text,
    /// Hash of the validated command body — the idempotency comparison key.
    #[column(not_null)]
    pub request_digest: Text,
    /// Epoch milliseconds the row was written.
    #[column(not_null, default = "unixepoch() * 1000")]
    pub created_at: Integer,
    /// Epoch milliseconds of the last change.
    #[column(not_null, default = "unixepoch() * 1000")]
    pub updated_at: Integer,
}

/// `project_approvals`: the one review decision on a proposal; a second
/// decision is a unique-constraint violation, not a race.
#[table(name = "project_approvals")]
#[unique_index("project_approvals_proposal_uidx", proposal_id)]
pub struct ProjectApprovals {
    /// UUID minted by the command core.
    #[column(primary_key)]
    pub id: Text,
    /// Owning project; deleting it cascades here.
    #[column(not_null, references = "projects(id)", on_delete = "cascade")]
    pub project_id: Text,
    /// Proposal decided; unique, so it has one decision.
    #[column(
        not_null,
        references = "project_plan_proposals(id)",
        on_delete = "cascade"
    )]
    pub proposal_id: Text,
    /// `approve`, `request_changes` or `reject`.
    #[column(not_null)]
    pub decision: Text,
    /// Replaces the platform's `reviewer_id`.
    #[column(not_null)]
    pub reviewer_principal_type: Text,
    /// Principal id of the reviewer.
    #[column(not_null)]
    pub reviewer_principal_id: Text,
    /// Reviewer's note.
    pub rationale: Text,
    /// Epoch milliseconds the row was written.
    #[column(not_null, default = "unixepoch() * 1000")]
    pub created_at: Integer,
}

#[table(name = "project_evidence")]
#[foreign_key(
    name = "project_evidence_work_item_project_fk",
    columns(work_item_id, project_id),
    references = "project_work_items(id, project_id)",
    on_delete = "cascade"
)]
#[foreign_key(
    name = "project_evidence_execution_project_fk",
    columns(execution_id, project_id),
    references = "project_executions(id, project_id)",
    on_delete = "cascade"
)]
/// `project_evidence`: one typed evidence record, attached to an item and an
/// execution of its own project, or (both `NULL`) to the project itself.
pub struct ProjectEvidence {
    /// UUID minted by the command core.
    #[column(primary_key)]
    pub id: Text,
    /// Owning project; deleting it cascades here.
    #[column(not_null, references = "projects(id)", on_delete = "cascade")]
    pub project_id: Text,
    /// Item the evidence supports, in the same project; `NULL` for the project.
    pub work_item_id: Text,
    /// Execution that produced it, in the same project, if any.
    pub execution_id: Text,
    /// Evidence kind (`pull_request`, `test_run`, …).
    #[column(not_null)]
    pub kind: Text,
    /// The system the evidence came from.
    #[column(not_null)]
    pub source: Text,
    /// Id in the source system.
    pub external_id: Text,
    /// Link to the evidence.
    pub url: Text,
    /// Verification state (`pending`, …).
    #[column(not_null, default = "'pending'")]
    pub trust: Text,
    /// JSON object.
    #[column(not_null, default = "'{}'")]
    pub metadata: Text,
    /// Hash of the evidence content, when known.
    pub content_hash: Text,
    /// The verifier's identity — a principal id or a system label such as
    /// `engine`; not a principal pair, as on the platform.
    pub verified_by: Text,
    /// Epoch milliseconds it was verified.
    pub verified_at: Integer,
    /// Principal kind of the creator.
    #[column(not_null)]
    pub creator_principal_type: Text,
    /// Principal id of the creator.
    #[column(not_null)]
    pub creator_principal_id: Text,
    /// Epoch milliseconds the row was written.
    #[column(not_null, default = "unixepoch() * 1000")]
    pub created_at: Integer,
}

/// `project_evidence_criteria`: which criteria a piece of evidence may satisfy.
#[table(name = "project_evidence_criteria")]
#[primary_key(evidence_id, criterion_id)]
pub struct ProjectEvidenceCriteria {
    /// The evidence.
    #[column(not_null, references = "project_evidence(id)", on_delete = "cascade")]
    pub evidence_id: Text,
    /// A criterion it may satisfy.
    #[column(not_null, references = "project_criteria(id)", on_delete = "cascade")]
    pub criterion_id: Text,
}

/// `project_activity_events`: one append-only event per mutation, written in
/// the mutation's transaction and paged by `(project_id, created_at, id)`.
#[table(name = "project_activity_events")]
#[index(
    "project_activity_events_project_created_idx",
    project_id,
    created_at,
    id
)]
pub struct ProjectActivityEvents {
    /// UUID minted by the command core.
    #[column(primary_key)]
    pub id: Text,
    /// Owning project; deleting it cascades here.
    #[column(not_null, references = "projects(id)", on_delete = "cascade")]
    pub project_id: Text,
    /// Principal kind of the actor.
    #[column(not_null)]
    pub actor_principal_type: Text,
    /// Principal id of the actor.
    #[column(not_null)]
    pub actor_principal_id: Text,
    /// What happened (`project.created`, …).
    #[column(not_null)]
    pub event_type: Text,
    /// Kind of entity it happened to.
    #[column(not_null)]
    pub entity_type: Text,
    /// Id of that entity.
    #[column(not_null)]
    pub entity_id: Text,
    /// JSON object.
    #[column(not_null, default = "'{}'")]
    pub payload: Text,
    /// Epoch milliseconds the row was written.
    #[column(not_null, default = "unixepoch() * 1000")]
    pub created_at: Integer,
}

#[table(name = "project_command_receipts")]
#[unique_index(
    "project_command_receipts_principal_key_uidx",
    principal_type,
    principal_id,
    idempotency_key
)]
/// `project_command_receipts`: an idempotent command's stored response,
/// scoped to the principal and key; kept until the project is hard-deleted.
pub struct ProjectCommandReceipts {
    /// UUID minted by the command core.
    #[column(primary_key)]
    pub id: Text,
    /// Owning project; deleting it cascades here.
    #[column(not_null, references = "projects(id)", on_delete = "cascade")]
    pub project_id: Text,
    /// Principal kind that issued the command.
    #[column(not_null)]
    pub principal_type: Text,
    /// Principal id that issued the command.
    #[column(not_null)]
    pub principal_id: Text,
    /// Caller-chosen key, unique per principal.
    #[column(not_null)]
    pub idempotency_key: Text,
    /// Command the receipt answers.
    #[column(not_null)]
    pub command_type: Text,
    /// Hash of the validated command body — replay versus conflict.
    #[column(not_null)]
    pub request_digest: Text,
    /// JSON response body, replayed byte for byte.
    #[column(not_null, default = "'{}'")]
    pub response: Text,
    /// Epoch milliseconds the row was written.
    #[column(not_null, default = "unixepoch() * 1000")]
    pub created_at: Integer,
}
