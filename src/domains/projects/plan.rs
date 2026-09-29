//! `project plan show` / `GET /api/v1/projects/{id}/plan`: the committed plan
//! at the project's current version, ported from the platform's
//! `getProjectPlan` and `toProjectPlanView` (`project-plan-service.ts`,
//! `project-plan-view.ts`), key for key and in its order.
//!
//! Archived milestones, work items and criteria are absent: archive is how an
//! approved proposal removes something, so listing them would contradict the
//! diff the reviewer approved. An edge naming an archived item is dropped
//! too, so the graph only names items the same response lists. A project
//! before its first approval reads plan version `0` with four empty arrays.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::domains::projects::authority::{Actor, Command, Verb, sealed};
use crate::domains::projects::view::{self, CriterionView};
use crate::prelude::*;
use crate::store::project_plan::{self, MilestoneRow, PlanRows, WorkItemRow};
use crate::store::project_read;
use crate::utilities::context::Ctx;
use crate::utilities::project_error::ProjectError;
use crate::utilities::uuid;

/// `project plan show` / `GET /api/v1/projects/{id}/plan` request.
#[derive(Deserialize, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// The project's UUID.
    pub id: String,
}

/// The committed plan.
#[derive(Serialize, Debug)]
pub struct Response {
    /// The plan at the project's current version.
    pub plan: PlanView,
}

/// The committed plan at one version.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanView {
    /// The project's UUID.
    pub project_id: String,
    /// The approved plan version; `0` before the first approval.
    pub plan_version: i64,
    /// Live milestones by `(position, id)`.
    pub milestones: Vec<MilestoneView>,
    /// Live work items by `(position, number)`.
    pub work_items: Vec<WorkItemView>,
    /// Live criteria of both levels by `(position, id)`.
    pub criteria: Vec<PlanCriterionView>,
    /// `blocks` edges whose two items are both live.
    pub dependencies: Vec<DependencyView>,
}

/// One milestone in the committed plan.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MilestoneView {
    /// UUID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Free text.
    pub description: String,
    /// ISO-8601.
    pub target_date: String,
    /// Display order.
    pub position: i64,
    /// Milestone status.
    pub status: String,
}

/// One work item in the committed plan, at its own optimistic `version`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkItemView {
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
}

/// A criterion plus the item it belongs to — `None` for a project-level one.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanCriterionView {
    /// The fields every criterion view carries.
    #[serde(flatten)]
    pub criterion: CriterionView,
    /// Owning item; `None` for a project-level criterion.
    pub work_item_id: Option<String>,
}

/// One directed `blocks` edge.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DependencyView {
    /// Item that blocks.
    pub blocker_id: String,
    /// Item that is blocked.
    pub blocked_id: String,
}

impl sealed::Sealed for Request {}

impl Command for Request {
    type Response = Response;

    fn verb(&self) -> Verb {
        Verb::PlanRead
    }

    /// Read the plan of the project `self.id` names.
    fn execute(self, ctx: &mut Ctx<'_>, _actor: &Actor) -> Result<Response> {
        let id = uuid::canonical(&self.id)
            .ok_or_else(|| Error::from(ProjectError::invalid_field("projectId", "invalid")))?;
        let conn = ctx.conn()?;
        let project = project_read::project(conn, &id)?.ok_or_else(|| {
            Error::from(ProjectError::ProjectNotFound {
                project_id: self.id,
            })
        })?;
        let rows = project_plan::plan_rows(conn, &project.id)?;
        let plan = plan_view(project.id, project.current_plan_version, rows)?;
        Ok(Response { plan })
    }
}

/// Project `rows` onto the wire plan, dropping every archived milestone,
/// work item and criterion, and every edge naming an archived item.
pub fn plan_view(project_id: String, plan_version: i64, rows: PlanRows) -> Result<PlanView> {
    let milestones = rows
        .milestones
        .into_iter()
        .filter(|m| m.archived_at.is_none())
        .map(milestone)
        .collect::<Result<_>>()?;
    let work_items: Vec<WorkItemView> = rows
        .work_items
        .into_iter()
        .filter(|w| w.archived_at.is_none())
        .map(work_item)
        .collect();
    let live: HashSet<&str> = work_items.iter().map(|w| w.id.as_str()).collect();
    let dependencies = rows
        .dependencies
        .into_iter()
        .filter(|d| live.contains(d.blocker_id.as_str()) && live.contains(d.blocked_id.as_str()))
        .map(|d| DependencyView {
            blocker_id: d.blocker_id,
            blocked_id: d.blocked_id,
        })
        .collect();
    let criteria = rows
        .criteria
        .into_iter()
        .filter(|c| c.archived_at.is_none())
        .map(|c| PlanCriterionView {
            criterion: view::criterion(c.criterion),
            work_item_id: c.work_item_id,
        })
        .collect();
    Ok(PlanView {
        project_id,
        plan_version,
        milestones,
        work_items,
        criteria,
        dependencies,
    })
}

/// One stored milestone as its view.
fn milestone(row: MilestoneRow) -> Result<MilestoneView> {
    let target_date = view::rendered(row.target_date, &row.id, "project_milestones.target_date")?;
    Ok(MilestoneView {
        id: row.id,
        name: row.name,
        description: row.description,
        target_date,
        position: row.position,
        status: row.status,
    })
}

/// One stored work item as its view.
fn work_item(row: WorkItemRow) -> WorkItemView {
    WorkItemView {
        id: row.id,
        number: row.number,
        parent_work_item_id: row.parent_work_item_id,
        milestone_id: row.milestone_id,
        kind: row.kind,
        title: row.title,
        description: row.description,
        status: row.status,
        priority: row.priority,
        estimate: row.estimate,
        assignee_principal_type: row.assignee_principal_type,
        assignee_principal_id: row.assignee_principal_id,
        repo: row.repo,
        version: row.version,
        position: row.position,
    }
}

#[cfg(test)]
#[path = "tests/plan.rs"]
mod tests;
