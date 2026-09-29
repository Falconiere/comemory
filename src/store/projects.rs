//! `projects` charter writes (#326): the project row with its unique-index
//! outcome classified, the repository and project-level criterion rows
//! written beside it, and the version-guarded lifecycle patch (#328). The
//! caller owns the transaction, so a later failure in the same command rolls
//! all of them back. Reads are
//! [`super::project_read`]; activity events [`super::project_activity`].

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_projects::{
    ProjectCriteria, ProjectRepositories, Projects, project_criteria as crit,
    project_repositories as repo, projects as col,
};
use crate::prelude::*;

/// One new `projects` row. Status, health, versions and the completion
/// policy take their declared defaults.
pub struct NewProject<'a> {
    /// Client-supplied or minted UUID.
    pub id: &'a str,
    /// Slug to try; the caller retries another on [`ProjectInsert::SlugTaken`].
    pub slug: &'a str,
    /// Work-item key prefix.
    pub key_prefix: &'a str,
    /// Display name.
    pub name: &'a str,
    /// The finite outcome.
    pub outcome: &'a str,
    /// JSON `string[]`.
    pub constraints: &'a str,
    /// JSON `string[]`.
    pub non_goals: &'a str,
    /// Lead principal kind.
    pub lead_type: &'a str,
    /// Lead principal id.
    pub lead_id: &'a str,
    /// Epoch milliseconds, when the charter names a deadline.
    pub target_date: Option<i64>,
    /// Creator principal kind.
    pub creator_type: &'a str,
    /// Creator principal id.
    pub creator_id: &'a str,
    /// Epoch milliseconds for both `created_at` and `updated_at`.
    pub at_ms: i64,
}

/// What one insert attempt did. Each `*Taken` is the unique index SQLite
/// reported; when a row breaks both, SQLite names the key prefix, so a
/// duplicate prefix is refused without a slug retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectInsert {
    /// The row was written.
    Inserted,
    /// Another project holds this slug.
    SlugTaken,
    /// Another project holds this key prefix.
    KeyPrefixTaken,
    /// Another project holds this id.
    IdTaken,
}

/// Insert `row`, turning a unique-index violation on `id`, `slug` or
/// `key_prefix` into its [`ProjectInsert`]; every other error propagates.
pub fn insert_project(conn: &Connection, row: &NewProject<'_>) -> Result<ProjectInsert> {
    let written = orm::execute(
        conn,
        Projects::insert()
            .set(&col::id, row.id)
            .set(&col::slug, row.slug)
            .set(&col::key_prefix, row.key_prefix)
            .set(&col::name, row.name)
            .set(&col::outcome, row.outcome)
            .set(&col::constraints, row.constraints)
            .set(&col::non_goals, row.non_goals)
            .set(&col::lead_principal_type, row.lead_type)
            .set(&col::lead_principal_id, row.lead_id)
            .set(&col::target_date, row.target_date)
            .set(&col::creator_principal_type, row.creator_type)
            .set(&col::creator_principal_id, row.creator_id)
            .set(&col::created_at, row.at_ms)
            .set(&col::updated_at, row.at_ms)
            .to_sql(),
    );
    match written {
        Ok(_) => Ok(ProjectInsert::Inserted),
        Err(e) => unique_violation(&e).ok_or(e),
    }
}

/// The [`ProjectInsert`] a `projects` unique-index violation stands for,
/// read from SQLite's `UNIQUE constraint failed: projects.<column>` message.
fn unique_violation(e: &Error) -> Option<ProjectInsert> {
    let Error::Sqlite(rusqlite::Error::SqliteFailure(ffi, Some(message))) = e else {
        return None;
    };
    let unique = matches!(
        ffi.extended_code,
        rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE | rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
    );
    match message.strip_prefix("UNIQUE constraint failed: ") {
        Some("projects.slug") if unique => Some(ProjectInsert::SlugTaken),
        Some("projects.key_prefix") if unique => Some(ProjectInsert::KeyPrefixTaken),
        Some("projects.id") if unique => Some(ProjectInsert::IdTaken),
        _ => None,
    }
}

/// The rows written beside a new project: the repositories it may use, in
/// order, and its project-level success criteria (`work_item_id` `NULL`,
/// `position` their index), each criterion an `(id, description)` pair.
pub fn insert_relations(
    conn: &Connection,
    project: &NewProject<'_>,
    repositories: &[String],
    criteria: &[(String, String)],
) -> Result<()> {
    // Each statement's initial values only fix its placeholder shape; every
    // row rebinds all of them, in the same order.
    let repository_row = ProjectRepositories::insert()
        .set(&repo::project_id, "")
        .set(&repo::repo, "")
        .set(&repo::creator_principal_type, "")
        .set(&repo::creator_principal_id, "")
        .set(&repo::created_at, 0_i64);
    orm::execute_many(
        conn,
        repository_row.to_sql(),
        repositories.iter().map(|name| {
            let creator = (project.creator_type, project.creator_id);
            (
                project.id,
                name.as_str(),
                creator.0,
                creator.1,
                project.at_ms,
            )
        }),
    )?;
    let criterion_row = ProjectCriteria::insert()
        .set(&crit::id, "")
        .set(&crit::project_id, "")
        .set(&crit::description, "")
        .set(&crit::position, 0_i64);
    orm::execute_many(
        conn,
        criterion_row.to_sql(),
        criteria
            .iter()
            .enumerate()
            .map(|(position, (id, description))| {
                (
                    id.as_str(),
                    project.id,
                    description.as_str(),
                    position as i64,
                )
            }),
    )?;
    Ok(())
}

/// What a lifecycle patch does to `archived_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchivedAt {
    /// Leave it as it is.
    Keep,
    /// Set it to these epoch milliseconds.
    Set(i64),
    /// Clear it to `NULL`.
    Clear,
}

/// A lifecycle command's change to one `projects` row: the new status when
/// it moves, what happens to `archived_at`, and the `updated_at` stamp.
pub struct LifecyclePatch<'a> {
    /// New `status`, or `None` to leave it.
    pub status: Option<&'a str>,
    /// The `archived_at` change.
    pub archived_at: ArchivedAt,
    /// Epoch milliseconds for `updated_at`.
    pub at_ms: i64,
}

/// Apply `patch` to project `id` only while it is still at `version`, moving
/// it to `version + 1`. `false` when nothing matched: the id is gone or
/// another writer moved the row since it was read, so the caller answers
/// `version_conflict` instead of overwriting the winner.
pub fn update_lifecycle(
    conn: &Connection,
    id: &str,
    version: i64,
    patch: &LifecyclePatch<'_>,
) -> Result<bool> {
    let mut query = Projects::update()
        .set(&col::version, version + 1)
        .set(&col::updated_at, patch.at_ms)
        .filter(col::id.eq(id).and(col::version.eq(version)));
    if let Some(status) = patch.status {
        query = query.set(&col::status, status);
    }
    match patch.archived_at {
        ArchivedAt::Keep => {}
        ArchivedAt::Set(at) => query = query.set(&col::archived_at, Some(at)),
        ArchivedAt::Clear => query = query.set(&col::archived_at, None::<i64>),
    }
    Ok(orm::execute(conn, query.to_sql())? == 1)
}

#[cfg(test)]
#[path = "tests/projects.rs"]
mod tests;
