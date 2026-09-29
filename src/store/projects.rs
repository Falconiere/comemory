//! `projects` charter writes (#326): the project row with its unique-index
//! outcome classified, and the repository and project-level criterion rows
//! written beside it. The caller owns the transaction, so a later failure in
//! the same command rolls all of them back. Reads are
//! [`super::project_read`]; activity events [`super::project_activity`].

use rusqlite::Connection;

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

/// The repositories a project may use, in the given order.
pub fn insert_repositories(
    conn: &Connection,
    project: &NewProject<'_>,
    repositories: &[String],
) -> Result<()> {
    for name in repositories {
        orm::execute(
            conn,
            ProjectRepositories::insert()
                .set(&repo::project_id, project.id)
                .set(&repo::repo, name.as_str())
                .set(&repo::creator_principal_type, project.creator_type)
                .set(&repo::creator_principal_id, project.creator_id)
                .set(&repo::created_at, project.at_ms)
                .to_sql(),
        )?;
    }
    Ok(())
}

/// Project-level success criteria (`work_item_id` `NULL`), `position` their
/// index; each `(id, description)` pair is one row with the declared defaults.
pub fn insert_criteria(
    conn: &Connection,
    project_id: &str,
    criteria: &[(String, String)],
) -> Result<()> {
    for (position, (id, description)) in criteria.iter().enumerate() {
        orm::execute(
            conn,
            ProjectCriteria::insert()
                .set(&crit::id, id.as_str())
                .set(&crit::project_id, project_id)
                .set(&crit::description, description.as_str())
                .set(&crit::position, position as i64)
                .to_sql(),
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/projects.rs"]
mod tests;
