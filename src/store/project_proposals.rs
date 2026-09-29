//! `project_plan_proposals` (#336): the immutable proposal row, the one
//! status move its first submission makes (`draft` → `planning`), and the
//! two reads — one proposal of one project, and a keyset page over
//! `(created_at DESC, id DESC)` — each with its one review decision from
//! `project_approvals`, when it has one.

use rusqlite::{Connection, Row};
use toolu_orm::core::expr::Scalar;
use toolu_orm::core::query_column::{ColumnRef, CommonOps, NumericOps};
use toolu_orm::query::select::SelectBuilder;

use super::orm;
use super::schema_project_record::{
    ProjectPlanProposals, project_approvals as review, project_plan_proposals as col,
};
use super::schema_projects::{Projects, projects};
use crate::prelude::*;

/// One new proposal row; its state takes the declared `pending` default.
pub struct NewProposal<'a> {
    /// Minted UUID.
    pub id: &'a str,
    /// Owning project.
    pub project_id: &'a str,
    /// The plan version the operations were written against.
    pub base_plan_version: i64,
    /// JSON `ProjectPlanOperation[]`, normalized.
    pub operations: &'a str,
    /// JSON `string[]`.
    pub assumptions: &'a str,
    /// JSON `string[]`.
    pub risks: &'a str,
    /// Why the change is proposed.
    pub rationale: &'a str,
    /// Proposer principal kind.
    pub proposer_type: &'a str,
    /// Proposer principal id.
    pub proposer_id: &'a str,
    /// The command's idempotency digest.
    pub request_digest: &'a str,
    /// Epoch milliseconds for both `created_at` and `updated_at`.
    pub at_ms: i64,
}

/// One stored proposal and its review decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalRow {
    /// UUID.
    pub id: String,
    /// Owning project.
    pub project_id: String,
    /// The plan version it was written against.
    pub base_plan_version: i64,
    /// `pending`, `approved`, `changes_requested`, `rejected` or `superseded`.
    pub state: String,
    /// Stored JSON operations text.
    pub operations: String,
    /// Stored JSON `string[]` text.
    pub assumptions: String,
    /// Stored JSON `string[]` text.
    pub risks: String,
    /// Why the change is proposed.
    pub rationale: String,
    /// Proposer principal kind.
    pub proposer_principal_type: String,
    /// Proposer principal id.
    pub proposer_principal_id: String,
    /// Epoch milliseconds.
    pub created_at: i64,
    /// Epoch milliseconds.
    pub updated_at: i64,
    /// The one `project_approvals` row, when it has been reviewed.
    pub review: Option<ReviewRow>,
}

/// A proposal's review decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRow {
    /// `approve`, `request_changes` or `reject`.
    pub decision: String,
    /// Reviewer principal id.
    pub reviewer_principal_id: String,
    /// Reviewer's note.
    pub rationale: Option<String>,
    /// Epoch milliseconds.
    pub created_at: i64,
}

/// What one page narrows to. `after` is the decoded keyset cursor.
#[derive(Debug, Clone, Copy)]
pub struct ProposalPage<'a> {
    /// Owning project.
    pub project_id: &'a str,
    /// Only this state.
    pub state: Option<&'a str>,
    /// Rows strictly after this `(created_at, id)`, newest first.
    pub after: Option<(i64, &'a str)>,
    /// Rows to return.
    pub limit: i64,
}

/// Write `row`.
pub fn insert(conn: &Connection, row: &NewProposal<'_>) -> Result<()> {
    orm::execute(
        conn,
        ProjectPlanProposals::insert()
            .set(&col::id, row.id)
            .set(&col::project_id, row.project_id)
            .set(&col::base_plan_version, row.base_plan_version)
            .set(&col::operations, row.operations)
            .set(&col::assumptions, row.assumptions)
            .set(&col::risks, row.risks)
            .set(&col::rationale, row.rationale)
            .set(&col::proposer_principal_type, row.proposer_type)
            .set(&col::proposer_principal_id, row.proposer_id)
            .set(&col::request_digest, row.request_digest)
            .set(&col::created_at, row.at_ms)
            .set(&col::updated_at, row.at_ms)
            .to_sql(),
    )?;
    Ok(())
}

/// Move a `draft` project to `planning`, bumping its optimistic `version`;
/// `false` (and no write) when it is in any other status.
pub fn start_planning(conn: &Connection, project_id: &str, at_ms: i64) -> Result<bool> {
    let changed = orm::execute(
        conn,
        Projects::update()
            .set(&projects::status, "planning")
            .set_expr(&projects::version, "version + 1")
            .set(&projects::updated_at, at_ms)
            .filter(projects::id.eq(project_id))
            .filter(projects::status.eq("draft"))
            .to_sql(),
    )?;
    Ok(changed == 1)
}

/// The proposal `id` of project `project_id`, when it exists there.
pub fn proposal(conn: &Connection, project_id: &str, id: &str) -> Result<Option<ProposalRow>> {
    let query = select_rows()
        .filter(col::project_id.eq(project_id))
        .filter(col::id.eq(id));
    orm::query_optional(conn, query.to_sql(), proposal_row)
}

/// Up to `page.limit` proposals of one project, newest first by
/// `(created_at, id)`.
pub fn proposal_page(conn: &Connection, page: &ProposalPage<'_>) -> Result<Vec<ProposalRow>> {
    let mut query = select_rows().filter(col::project_id.eq(page.project_id));
    if let Some(state) = page.state {
        query = query.filter(col::state.eq(state));
    }
    if let Some((at_ms, id)) = page.after {
        let tie = col::created_at
            .eq(at_ms)
            .and(Scalar::col(&col::id).lt(Scalar::bind(id)));
        query = query.filter(col::created_at.lt(at_ms).or(tie));
    }
    let query = query
        .order_by(col::created_at.desc())
        .order_by(col::id.desc())
        .limit(page.limit);
    orm::query_all(conn, query.to_sql(), proposal_row)
}

/// Every proposal column, then its review's, left-joined on `proposal_id`.
fn select_rows() -> SelectBuilder {
    let proposal: [&dyn ColumnRef; 12] = [
        &col::id,
        &col::project_id,
        &col::base_plan_version,
        &col::state,
        &col::operations,
        &col::assumptions,
        &col::risks,
        &col::rationale,
        &col::proposer_principal_type,
        &col::proposer_principal_id,
        &col::created_at,
        &col::updated_at,
    ];
    let reviewed: [&dyn ColumnRef; 4] = [
        &review::decision,
        &review::reviewer_principal_id,
        &review::rationale,
        &review::created_at,
    ];
    let mut query = ProjectPlanProposals::select().columns_typed(&[]);
    for (index, column) in proposal.into_iter().chain(reviewed).enumerate() {
        let qualified = format!("{}.{}", column.table(), column.name());
        query = query.column_expr(&qualified, &format!("c{index}"));
    }
    query.left_join("project_approvals", review::proposal_id.equals(&col::id))
}

/// Decode one [`select_rows`] row.
fn proposal_row(r: &Row<'_>) -> rusqlite::Result<ProposalRow> {
    let decision: Option<String> = r.get(12)?;
    let review = match decision {
        Some(decision) => Some(ReviewRow {
            decision,
            reviewer_principal_id: r.get(13)?,
            rationale: r.get(14)?,
            created_at: r.get(15)?,
        }),
        None => None,
    };
    Ok(ProposalRow {
        id: r.get(0)?,
        project_id: r.get(1)?,
        base_plan_version: r.get(2)?,
        state: r.get(3)?,
        operations: r.get(4)?,
        assumptions: r.get(5)?,
        risks: r.get(6)?,
        rationale: r.get(7)?,
        proposer_principal_type: r.get(8)?,
        proposer_principal_id: r.get(9)?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
        review,
    })
}

#[cfg(test)]
#[path = "tests/project_proposals.rs"]
mod tests;
