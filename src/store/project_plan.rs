//! A project's plan rows (#335): milestones, work items, criteria of both
//! levels and `blocks` dependencies, ported from the platform's
//! `loadPlanRows`. Archived rows come back too — the plan view drops them,
//! while a proposal's preflight and diff must tell an archived entity from an
//! unknown one.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::project_read::CriterionRow;
use super::schema_project_work::{
    ProjectWorkItemDependencies, ProjectWorkItems, project_work_item_dependencies as dep,
    project_work_items as item,
};
use super::schema_projects::{
    ProjectCriteria, ProjectMilestones, project_criteria as crit, project_milestones as ms,
};
use crate::prelude::*;

/// One stored `project_milestones` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MilestoneRow {
    /// UUID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Free text.
    pub description: String,
    /// Epoch milliseconds.
    pub target_date: i64,
    /// Display order.
    pub position: i64,
    /// Milestone status.
    pub status: String,
    /// Epoch milliseconds, when archived.
    pub archived_at: Option<i64>,
}

/// One stored `project_work_items` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemRow {
    /// UUID.
    pub id: String,
    /// Per-project display number.
    pub number: i64,
    /// Parent item, if nested.
    pub parent_work_item_id: Option<String>,
    /// Milestone, if any.
    pub milestone_id: Option<String>,
    /// Item kind.
    pub kind: String,
    /// One-line title.
    pub title: String,
    /// Free text.
    pub description: String,
    /// Item status.
    pub status: String,
    /// Priority.
    pub priority: String,
    /// Size estimate, if given.
    pub estimate: Option<i64>,
    /// Assignee principal kind.
    pub assignee_principal_type: Option<String>,
    /// Assignee principal id.
    pub assignee_principal_id: Option<String>,
    /// Canonical `owner/name`, if scoped to one repository.
    pub repo: Option<String>,
    /// Optimistic-concurrency counter.
    pub version: i64,
    /// Display order.
    pub position: i64,
    /// Epoch milliseconds, when archived.
    pub archived_at: Option<i64>,
}

/// One stored `project_criteria` row of either level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanCriterionRow {
    /// The fields a project-level criterion shares.
    pub criterion: CriterionRow,
    /// Owning item; `None` for a project-level criterion.
    pub work_item_id: Option<String>,
    /// Epoch milliseconds, when archived.
    pub archived_at: Option<i64>,
}

/// One `blocks` edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyRow {
    /// Item that blocks.
    pub blocker_id: String,
    /// Item that is blocked.
    pub blocked_id: String,
}

/// Every stored plan row of one project, archived rows included.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanRows {
    /// By `(position, id)`.
    pub milestones: Vec<MilestoneRow>,
    /// By `(position, number)`.
    pub work_items: Vec<WorkItemRow>,
    /// By `(position, id)`, both levels.
    pub criteria: Vec<PlanCriterionRow>,
    /// By `(blocker_id, blocked_id)`; the platform leaves them unordered.
    pub dependencies: Vec<DependencyRow>,
}

/// The plan rows of `project_id`, one query per table.
pub fn plan_rows(conn: &Connection, project_id: &str) -> Result<PlanRows> {
    let milestones = ProjectMilestones::select()
        .columns_typed(&[
            &ms::id,
            &ms::name,
            &ms::description,
            &ms::target_date,
            &ms::position,
            &ms::status,
            &ms::archived_at,
        ])
        .filter(ms::project_id.eq(project_id))
        .order_by(ms::position.asc())
        .order_by(ms::id.asc());
    let work_items = ProjectWorkItems::select()
        .columns_typed(&[
            &item::id,
            &item::number,
            &item::parent_work_item_id,
            &item::milestone_id,
            &item::kind,
            &item::title,
            &item::description,
            &item::status,
            &item::priority,
            &item::estimate,
            &item::assignee_principal_type,
            &item::assignee_principal_id,
            &item::repo,
            &item::version,
            &item::position,
            &item::archived_at,
        ])
        .filter(item::project_id.eq(project_id))
        .order_by(item::position.asc())
        .order_by(item::number.asc());
    let milestones = orm::query_all(conn, milestones.to_sql(), |r| {
        Ok(MilestoneRow {
            id: r.get(0)?,
            name: r.get(1)?,
            description: r.get(2)?,
            target_date: r.get(3)?,
            position: r.get(4)?,
            status: r.get(5)?,
            archived_at: r.get(6)?,
        })
    })?;
    let work_items = orm::query_all(conn, work_items.to_sql(), |r| {
        Ok(WorkItemRow {
            id: r.get(0)?,
            number: r.get(1)?,
            parent_work_item_id: r.get(2)?,
            milestone_id: r.get(3)?,
            kind: r.get(4)?,
            title: r.get(5)?,
            description: r.get(6)?,
            status: r.get(7)?,
            priority: r.get(8)?,
            estimate: r.get(9)?,
            assignee_principal_type: r.get(10)?,
            assignee_principal_id: r.get(11)?,
            repo: r.get(12)?,
            version: r.get(13)?,
            position: r.get(14)?,
            archived_at: r.get(15)?,
        })
    })?;
    let (criteria, dependencies) = criteria_and_edges(conn, project_id)?;
    Ok(PlanRows {
        milestones,
        work_items,
        criteria,
        dependencies,
    })
}

/// The criteria of both levels and the `blocks` edges of `project_id`.
fn criteria_and_edges(
    conn: &Connection,
    project_id: &str,
) -> Result<(Vec<PlanCriterionRow>, Vec<DependencyRow>)> {
    let edges = ProjectWorkItemDependencies::select()
        .columns_typed(&[&dep::blocker_id, &dep::blocked_id])
        .filter(dep::project_id.eq(project_id))
        .order_by(dep::blocker_id.asc())
        .order_by(dep::blocked_id.asc())
        .to_sql();
    let edges = orm::query_all(conn, edges, |r| {
        Ok(DependencyRow {
            blocker_id: r.get(0)?,
            blocked_id: r.get(1)?,
        })
    })?;
    let criteria = orm::query_all(
        conn,
        ProjectCriteria::select()
            .columns_typed(&[
                &crit::id,
                &crit::project_id,
                &crit::description,
                &crit::required,
                &crit::evidence_requirement,
                &crit::resolution,
                &crit::resolution_rationale,
                &crit::position,
                &crit::work_item_id,
                &crit::archived_at,
            ])
            .filter(crit::project_id.eq(project_id))
            .order_by(crit::position.asc())
            .order_by(crit::id.asc())
            .to_sql(),
        |r| {
            let criterion = CriterionRow {
                id: r.get(0)?,
                project_id: r.get(1)?,
                description: r.get(2)?,
                required: r.get::<_, i64>(3)? != 0,
                evidence_requirement: r.get(4)?,
                resolution: r.get(5)?,
                resolution_rationale: r.get(6)?,
                position: r.get(7)?,
            };
            Ok(PlanCriterionRow {
                criterion,
                work_item_id: r.get(8)?,
                archived_at: r.get(9)?,
            })
        },
    )?;
    Ok((criteria, edges))
}

#[cfg(test)]
#[path = "tests/project_plan.rs"]
mod tests;
