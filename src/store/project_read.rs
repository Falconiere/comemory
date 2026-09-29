//! `projects` charter reads (#326): one row by id, a keyset page over
//! `(created_at DESC, id DESC)`, and the repositories and project-level
//! criteria of a whole page in one query each.

use rusqlite::{Connection, Row};
use toolu_orm::core::expr::Scalar;
use toolu_orm::core::query_column::{CommonOps, NumericOps};
use toolu_orm::core::value::Value;
use toolu_orm::query::select::SelectBuilder;

use super::orm;
use super::schema_projects::{
    ProjectCriteria, ProjectRepositories, Projects, project_criteria as crit,
    project_repositories as repo, projects as col,
};
use crate::prelude::*;

/// One stored `projects` row, column for column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRow {
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
    /// Stored JSON `string[]` text.
    pub constraints: String,
    /// Stored JSON `string[]` text.
    pub non_goals: String,
    /// Lifecycle status.
    pub status: String,
    /// Reported health.
    pub health: String,
    /// Lead principal kind.
    pub lead_principal_type: String,
    /// Lead principal id.
    pub lead_principal_id: String,
    /// Epoch milliseconds, when set.
    pub target_date: Option<i64>,
    /// Approved plan version.
    pub current_plan_version: i64,
    /// Optimistic-concurrency counter.
    pub version: i64,
    /// How the project completes.
    pub completion_policy: String,
    /// Creator principal kind.
    pub creator_principal_type: String,
    /// Creator principal id.
    pub creator_principal_id: String,
    /// Epoch milliseconds.
    pub created_at: i64,
    /// Epoch milliseconds.
    pub updated_at: i64,
    /// Epoch milliseconds, when archived.
    pub archived_at: Option<i64>,
}

/// One project-level `project_criteria` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CriterionRow {
    /// UUID.
    pub id: String,
    /// Owning project.
    pub project_id: String,
    /// Free text.
    pub description: String,
    /// Stored 0/1.
    pub required: bool,
    /// Evidence policy.
    pub evidence_requirement: String,
    /// `open`, `accepted` or `waived`.
    pub resolution: String,
    /// Waiver rationale, when recorded.
    pub resolution_rationale: Option<String>,
    /// Display order.
    pub position: i64,
}

/// What one page narrows to. `after` is the decoded keyset cursor.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProjectPage<'a> {
    /// Only this status.
    pub status: Option<&'a str>,
    /// Only this health.
    pub health: Option<&'a str>,
    /// Archived rows too.
    pub include_archived: bool,
    /// Rows strictly after this `(created_at, id)`, newest first.
    pub after: Option<(i64, &'a str)>,
    /// Rows to return.
    pub limit: i64,
}

/// Every `projects` column, in [`ProjectRow`] order.
fn select_rows() -> SelectBuilder {
    Projects::select().columns_typed(&[
        &col::id,
        &col::slug,
        &col::key_prefix,
        &col::name,
        &col::outcome,
        &col::constraints,
        &col::non_goals,
        &col::status,
        &col::health,
        &col::lead_principal_type,
        &col::lead_principal_id,
        &col::target_date,
        &col::current_plan_version,
        &col::version,
        &col::completion_policy,
        &col::creator_principal_type,
        &col::creator_principal_id,
        &col::created_at,
        &col::updated_at,
        &col::archived_at,
    ])
}

/// Decode one [`select_rows`] row.
fn project_row(r: &Row<'_>) -> rusqlite::Result<ProjectRow> {
    Ok(ProjectRow {
        id: r.get(0)?,
        slug: r.get(1)?,
        key_prefix: r.get(2)?,
        name: r.get(3)?,
        outcome: r.get(4)?,
        constraints: r.get(5)?,
        non_goals: r.get(6)?,
        status: r.get(7)?,
        health: r.get(8)?,
        lead_principal_type: r.get(9)?,
        lead_principal_id: r.get(10)?,
        target_date: r.get(11)?,
        current_plan_version: r.get(12)?,
        version: r.get(13)?,
        completion_policy: r.get(14)?,
        creator_principal_type: r.get(15)?,
        creator_principal_id: r.get(16)?,
        created_at: r.get(17)?,
        updated_at: r.get(18)?,
        archived_at: r.get(19)?,
    })
}

/// The project with `id`, when one exists.
pub fn project(conn: &Connection, id: &str) -> Result<Option<ProjectRow>> {
    orm::query_optional(
        conn,
        select_rows().filter(col::id.eq(id)).to_sql(),
        project_row,
    )
}

/// Up to `page.limit` rows, newest first by `(created_at, id)`.
pub fn project_page(conn: &Connection, page: &ProjectPage<'_>) -> Result<Vec<ProjectRow>> {
    let mut query = select_rows();
    if let Some(status) = page.status {
        query = query.filter(col::status.eq(status));
    }
    if let Some(health) = page.health {
        query = query.filter(col::health.eq(health));
    }
    if !page.include_archived {
        query = query.filter(col::archived_at.is_null());
    }
    if let Some((at_ms, id)) = page.after {
        let same_ms_smaller_id = col::created_at
            .eq(at_ms)
            .and(Scalar::col(&col::id).lt(Scalar::bind(id)));
        query = query.filter(col::created_at.lt(at_ms).or(same_ms_smaller_id));
    }
    orm::query_all(
        conn,
        query
            .order_by(col::created_at.desc())
            .order_by(col::id.desc())
            .limit(page.limit)
            .to_sql(),
        project_row,
    )
}

/// `(project_id, repo)` for every project in `ids`, ordered by project then
/// repository — the order the platform's primary-key index yields.
pub fn repositories(conn: &Connection, ids: &[String]) -> Result<Vec<(String, String)>> {
    orm::query_all(
        conn,
        ProjectRepositories::select()
            .columns_typed(&[&repo::project_id, &repo::repo])
            .filter(repo::project_id.in_list(&values(ids)))
            .order_by(repo::project_id.asc())
            .order_by(repo::repo.asc())
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// Project-level criteria (`work_item_id IS NULL`) of every project in `ids`,
/// in display order. Archived criteria are included, as on the platform.
pub fn criteria(conn: &Connection, ids: &[String]) -> Result<Vec<CriterionRow>> {
    orm::query_all(
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
            ])
            .filter(crit::project_id.in_list(&values(ids)))
            .filter(crit::work_item_id.is_null())
            .order_by(crit::project_id.asc())
            .order_by(crit::position.asc())
            .to_sql(),
        |r| {
            Ok(CriterionRow {
                id: r.get(0)?,
                project_id: r.get(1)?,
                description: r.get(2)?,
                required: r.get::<_, i64>(3)? != 0,
                evidence_requirement: r.get(4)?,
                resolution: r.get(5)?,
                resolution_rationale: r.get(6)?,
                position: r.get(7)?,
            })
        },
    )
}

/// `ids` as bind values.
fn values(ids: &[String]) -> Vec<Value> {
    ids.iter().map(|id| id.as_str().into()).collect()
}

#[cfg(test)]
#[path = "tests/project_read.rs"]
mod tests;
