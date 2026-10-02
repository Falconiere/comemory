//! A plan proposal on the wire — the platform's `ProjectProposalView`
//! (`project-proposal-view.ts`), key for key and in its order — from a
//! stored row and its one review decision. An unreadable stored JSON column
//! reads as empty with a warning, as the platform's read path degrades it,
//! so one corrupt proposal cannot cost a whole page.

use serde::{Deserialize, Serialize};

use crate::domains::projects::operations::Operation;
use crate::domains::projects::view::{rendered, stored_json};
use crate::prelude::*;
use crate::store::project_proposals::{ProposalRow, ReviewRow};

/// One immutable proposal, with its review when it has one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProposalView {
    /// UUID.
    pub id: String,
    /// Owning project.
    pub project_id: String,
    /// The plan version it was written against.
    pub base_plan_version: i64,
    /// `pending`, `approved`, `changes_requested`, `rejected` or `superseded`.
    pub state: String,
    /// The normalized operations, in submission order.
    pub operations: Vec<Operation>,
    /// Why the change is proposed.
    pub rationale: String,
    /// What the proposer assumed.
    pub assumptions: Vec<String>,
    /// What could go wrong.
    pub risks: Vec<String>,
    /// `user` or `project_agent`.
    pub proposer_principal_type: String,
    /// The proposer's principal id.
    pub proposer_principal_id: String,
    /// ISO-8601.
    pub created_at: String,
    /// ISO-8601.
    pub updated_at: String,
    /// The review decision; `null` while pending.
    pub review: Option<ReviewView>,
}

/// A proposal's one review decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewView {
    /// `approve`, `request_changes` or `reject`.
    pub decision: String,
    /// The reviewer's principal id (the platform's `reviewerId`).
    pub reviewer_id: String,
    /// The reviewer's note.
    pub rationale: Option<String>,
    /// ISO-8601.
    pub created_at: String,
}

/// `row` as its wire view.
pub fn view(row: ProposalRow) -> Result<ProposalView> {
    let column = |name: &str| format!("project_plan_proposals.{name}");
    let review = row.review.map(|r| review(r, &row.id)).transpose()?;
    Ok(ProposalView {
        operations: stored_json(&row.operations, &row.id, &column("operations")),
        assumptions: stored_json(&row.assumptions, &row.id, &column("assumptions")),
        risks: stored_json(&row.risks, &row.id, &column("risks")),
        created_at: rendered(row.created_at, &row.id, &column("created_at"))?,
        updated_at: rendered(row.updated_at, &row.id, &column("updated_at"))?,
        id: row.id,
        project_id: row.project_id,
        base_plan_version: row.base_plan_version,
        state: row.state,
        rationale: row.rationale,
        proposer_principal_type: row.proposer_principal_type,
        proposer_principal_id: row.proposer_principal_id,
        review,
    })
}

/// A stored review of proposal `proposal_id` as its view.
fn review(row: ReviewRow, proposal_id: &str) -> Result<ReviewView> {
    Ok(ReviewView {
        created_at: rendered(row.created_at, proposal_id, "project_approvals.created_at")?,
        decision: row.decision,
        reviewer_id: row.reviewer_principal_id,
        rationale: row.rationale,
    })
}
