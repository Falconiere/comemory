//! The charter read shape every project command and read returns — the
//! platform's `ProjectView` (`project-view.ts`), key for key and in its
//! order, minus `workspaceId`: one data directory is one workspace.
//! `leadUserId` and `createdBy` carry the lead's and creator's principal
//! ids; both are always human (`user`) principals.

use std::collections::HashMap;

use serde::Serialize;

use crate::domains::projects::timestamp::iso;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::project_read::{self, CriterionRow, ProjectRow};

/// One project-level success criterion.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CriterionView {
    /// UUID.
    pub id: String,
    /// Free text.
    pub description: String,
    /// Whether completion needs it.
    pub required: bool,
    /// Evidence policy (`reported`, …).
    pub evidence_requirement: String,
    /// `open`, `accepted` or `waived`.
    pub resolution: String,
    /// Waiver rationale.
    pub resolution_rationale: Option<String>,
    /// Display order.
    pub position: i64,
}

/// The charter, its repositories and project-level criteria, and its
/// lifecycle state.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    /// UUID.
    pub id: String,
    /// URL slug.
    pub slug: String,
    /// Work-item key prefix.
    pub key_prefix: String,
    /// Display name.
    pub name: String,
    /// The finite outcome.
    pub outcome: String,
    /// Constraints.
    pub constraints: Vec<String>,
    /// Non-goals.
    pub non_goals: Vec<String>,
    /// Lifecycle status.
    pub status: String,
    /// Reported health.
    pub health: String,
    /// The lead's principal id.
    pub lead_user_id: String,
    /// ISO-8601 deadline, when set.
    pub target_date: Option<String>,
    /// Approved plan version.
    pub current_plan_version: i64,
    /// Optimistic-concurrency counter.
    pub version: i64,
    /// How the project completes.
    pub completion_policy: String,
    /// The creator's principal id.
    pub created_by: String,
    /// ISO-8601.
    pub created_at: String,
    /// ISO-8601.
    pub updated_at: String,
    /// ISO-8601, when archived.
    pub archived_at: Option<String>,
    /// Canonical `owner/name` repositories.
    pub repositories: Vec<String>,
    /// Project-level success criteria.
    pub criteria: Vec<CriterionView>,
}

/// The views of `rows`, in order, with their relations loaded in one query
/// each rather than two per row.
pub fn load(conn: &Connection, rows: Vec<ProjectRow>) -> Result<Vec<ProjectView>> {
    let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
    let mut repositories: HashMap<String, Vec<String>> = HashMap::new();
    for (project_id, repo) in project_read::repositories(conn, &ids)? {
        repositories.entry(project_id).or_default().push(repo);
    }
    let mut criteria: HashMap<String, Vec<CriterionView>> = HashMap::new();
    for row in project_read::criteria(conn, &ids)? {
        criteria
            .entry(row.project_id.clone())
            .or_default()
            .push(criterion(row));
    }
    Ok(rows
        .into_iter()
        .map(|row| {
            let repos = repositories.remove(&row.id).unwrap_or_default();
            let crits = criteria.remove(&row.id).unwrap_or_default();
            project(row, repos, crits)
        })
        .collect())
}

/// One stored criterion as its view.
fn criterion(row: CriterionRow) -> CriterionView {
    CriterionView {
        id: row.id,
        description: row.description,
        required: row.required,
        evidence_requirement: row.evidence_requirement,
        resolution: row.resolution,
        resolution_rationale: row.resolution_rationale,
        position: row.position,
    }
}

/// One stored row plus its relations as the view.
fn project(
    row: ProjectRow,
    repositories: Vec<String>,
    criteria: Vec<CriterionView>,
) -> ProjectView {
    let constraints = string_array(&row.constraints, &row.id, "projects.constraints");
    let non_goals = string_array(&row.non_goals, &row.id, "projects.non_goals");
    ProjectView {
        id: row.id,
        slug: row.slug,
        key_prefix: row.key_prefix,
        name: row.name,
        outcome: row.outcome,
        constraints,
        non_goals,
        status: row.status,
        health: row.health,
        lead_user_id: row.lead_principal_id,
        target_date: row.target_date.and_then(iso),
        current_plan_version: row.current_plan_version,
        version: row.version,
        completion_policy: row.completion_policy,
        created_by: row.creator_principal_id,
        created_at: iso(row.created_at).unwrap_or_default(),
        updated_at: iso(row.updated_at).unwrap_or_default(),
        archived_at: row.archived_at.and_then(iso),
        repositories,
        criteria,
    }
}

/// A stored JSON `string[]`; an unreadable value degrades to empty with a
/// warning, as the platform's `parseStoredJson` does.
fn string_array(raw: &str, row_id: &str, column: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_else(|e| {
        tracing::warn!(row_id, column, error = %e, "unreadable stored JSON; reading as []");
        Vec::new()
    })
}
