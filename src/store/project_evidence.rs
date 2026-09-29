//! `project_evidence` and `project_evidence_criteria` (#346): the typed
//! evidence row an attach writes inside its command's transaction, its
//! criterion links, the two membership reads the attach checks first, and the
//! keyset page over `(created_at, id)` newest first, filtered by kind, trust
//! and work item. Trust policy (which stored values read as `invalid`) is
//! `domains::projects::evidence`'s; this file only binds what it is handed.

use rusqlite::Connection;
use toolu_orm::core::expr::Scalar;
use toolu_orm::core::query_column::{CommonOps, NumericOps};
use toolu_orm::core::value::Value;

use super::orm;
use super::schema_project_record::{
    ProjectEvidence, ProjectEvidenceCriteria, project_evidence as col,
    project_evidence_criteria as link,
};
use super::schema_project_work::{ProjectWorkItems, project_work_items as item};
use super::schema_projects::{ProjectCriteria, project_criteria as crit};
use crate::prelude::*;

/// One new evidence row. `execution_id` and the verifier columns stay `NULL`
/// until #347 and #348 write them.
pub struct NewEvidence<'a> {
    /// Evidence UUID.
    pub id: &'a str,
    /// Owning project.
    pub project_id: &'a str,
    /// The work item it supports; `None` for the project itself.
    pub work_item_id: Option<&'a str>,
    /// Evidence kind.
    pub kind: &'a str,
    /// The system it came from.
    pub source: &'a str,
    /// Id in that system.
    pub external_id: Option<&'a str>,
    /// Link to it.
    pub url: Option<&'a str>,
    /// Initial trust.
    pub trust: &'a str,
    /// Encoded stored metadata.
    pub metadata: &'a str,
    /// Creator principal kind.
    pub creator_type: &'a str,
    /// Creator principal id.
    pub creator_id: &'a str,
    /// Epoch milliseconds.
    pub at_ms: i64,
}

/// One stored evidence row, column for column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRow {
    /// Evidence UUID.
    pub id: String,
    /// The work item it supports, if any.
    pub work_item_id: Option<String>,
    /// The execution that produced it, if any.
    pub execution_id: Option<String>,
    /// Kind, as stored.
    pub kind: String,
    /// Source system.
    pub source: String,
    /// Id in the source system.
    pub external_id: Option<String>,
    /// Link.
    pub url: Option<String>,
    /// Trust, as stored.
    pub trust: String,
    /// Stored JSON text.
    pub metadata: String,
    /// Content hash, when known.
    pub content_hash: Option<String>,
    /// Verifier identity.
    pub verified_by: Option<String>,
    /// Epoch milliseconds it was verified.
    pub verified_at: Option<i64>,
    /// Creator principal kind.
    pub creator_principal_type: String,
    /// Creator principal id.
    pub creator_principal_id: String,
    /// Epoch milliseconds.
    pub created_at: i64,
}

/// Which stored trust values a page keeps.
#[derive(Debug, Clone, Copy)]
pub enum TrustFilter<'a> {
    /// Exactly this value.
    Exactly(&'a str),
    /// Any value but these — the rows a reader sees as the least-trusted one.
    NoneOf(&'a [&'a str]),
}

/// One page of one project's evidence, newest first.
#[derive(Debug, Clone, Copy)]
pub struct EvidencePage<'a> {
    /// The project whose evidence is read.
    pub project_id: &'a str,
    /// Only this kind.
    pub kind: Option<&'a str>,
    /// Only these trust values.
    pub trust: Option<TrustFilter<'a>>,
    /// Only evidence on this work item.
    pub work_item_id: Option<&'a str>,
    /// Rows strictly older than this `(created_at, id)`.
    pub before: Option<(i64, &'a str)>,
    /// Rows to return.
    pub limit: i64,
}

/// Append `row`.
pub fn insert(conn: &Connection, row: &NewEvidence<'_>) -> Result<()> {
    orm::execute(
        conn,
        ProjectEvidence::insert()
            .set(&col::id, row.id)
            .set(&col::project_id, row.project_id)
            .set(&col::work_item_id, row.work_item_id)
            .set(&col::kind, row.kind)
            .set(&col::source, row.source)
            .set(&col::external_id, row.external_id)
            .set(&col::url, row.url)
            .set(&col::trust, row.trust)
            .set(&col::metadata, row.metadata)
            .set(&col::creator_principal_type, row.creator_type)
            .set(&col::creator_principal_id, row.creator_id)
            .set(&col::created_at, row.at_ms)
            .to_sql(),
    )?;
    Ok(())
}

/// Link `evidence_id` to each of `criterion_ids`, which must be distinct.
pub fn link_criteria(conn: &Connection, evidence_id: &str, criterion_ids: &[String]) -> Result<()> {
    let row = ProjectEvidenceCriteria::insert()
        .set(&link::evidence_id, "")
        .set(&link::criterion_id, "");
    orm::execute_many(
        conn,
        row.to_sql(),
        criterion_ids
            .iter()
            .map(|criterion| (evidence_id, criterion.as_str())),
    )?;
    Ok(())
}

/// Which of an attach's references belong to its project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Members {
    /// Whether the named work item is one of the project's, archived or not;
    /// `false` when none was named.
    pub work_item: bool,
    /// The named criterion ids that are the project's, at either level.
    pub criteria: Vec<String>,
}

/// The [`Members`] of `project_id` among `work_item_id` and `criterion_ids`:
/// one query per kind of reference, none for an absent one.
pub fn members(
    conn: &Connection,
    project_id: &str,
    work_item_id: Option<&str>,
    criterion_ids: &[String],
) -> Result<Members> {
    let work_item = match work_item_id {
        None => false,
        Some(id) => orm::query_optional(
            conn,
            ProjectWorkItems::select()
                .columns_typed(&[&item::id])
                .filter(item::project_id.eq(project_id))
                .filter(item::id.eq(id))
                .to_sql(),
            |r| r.get::<_, String>(0),
        )?
        .is_some(),
    };
    let criteria = if criterion_ids.is_empty() {
        Vec::new()
    } else {
        let values: Vec<Value> = criterion_ids.iter().map(|id| id.as_str().into()).collect();
        let query = ProjectCriteria::select()
            .columns_typed(&[&crit::id])
            .filter(crit::project_id.eq(project_id))
            .filter(crit::id.in_list(&values));
        orm::query_all(conn, query.to_sql(), |r| r.get(0))?
    };
    Ok(Members {
        work_item,
        criteria,
    })
}

/// The evidence row `id` of `project_id`, if any.
pub fn one(conn: &Connection, project_id: &str, id: &str) -> Result<Option<EvidenceRow>> {
    let query = select()
        .filter(col::project_id.eq(project_id))
        .filter(col::id.eq(id));
    orm::query_optional(conn, query.to_sql(), row)
}

/// Up to `page.limit` of the project's evidence ordered by `(created_at, id)`
/// newest first, every filter applied together.
pub fn page(conn: &Connection, page: &EvidencePage<'_>) -> Result<Vec<EvidenceRow>> {
    let mut query = select().filter(col::project_id.eq(page.project_id));
    if let Some(kind) = page.kind {
        query = query.filter(col::kind.eq(kind));
    }
    match page.trust {
        Some(TrustFilter::Exactly(trust)) => query = query.filter(col::trust.eq(trust)),
        Some(TrustFilter::NoneOf(known)) => {
            let known: Vec<Value> = known.iter().map(|t| (*t).into()).collect();
            query = query.filter(col::trust.not_in(&known));
        }
        None => {}
    }
    if let Some(work_item_id) = page.work_item_id {
        query = query.filter(col::work_item_id.eq(work_item_id));
    }
    if let Some((at_ms, id)) = page.before {
        let (id_col, id) = (Scalar::col(&col::id), Scalar::bind(id));
        query = query.filter(
            col::created_at
                .lt(at_ms)
                .or(col::created_at.eq(at_ms).and(id_col.lt(id))),
        );
    }
    let query = query
        .order_by(col::created_at.desc())
        .order_by(col::id.desc())
        .limit(page.limit);
    orm::query_all(conn, query.to_sql(), row)
}

/// Every [`EvidenceRow`] column, in its field order.
fn select() -> toolu_orm::query::select::SelectBuilder {
    ProjectEvidence::select().columns_typed(&[
        &col::id,
        &col::work_item_id,
        &col::execution_id,
        &col::kind,
        &col::source,
        &col::external_id,
        &col::url,
        &col::trust,
        &col::metadata,
        &col::content_hash,
        &col::verified_by,
        &col::verified_at,
        &col::creator_principal_type,
        &col::creator_principal_id,
        &col::created_at,
    ])
}

/// One [`select`] row.
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<EvidenceRow> {
    Ok(EvidenceRow {
        id: r.get(0)?,
        work_item_id: r.get(1)?,
        execution_id: r.get(2)?,
        kind: r.get(3)?,
        source: r.get(4)?,
        external_id: r.get(5)?,
        url: r.get(6)?,
        trust: r.get(7)?,
        metadata: r.get(8)?,
        content_hash: r.get(9)?,
        verified_by: r.get(10)?,
        verified_at: r.get(11)?,
        creator_principal_type: r.get(12)?,
        creator_principal_id: r.get(13)?,
        created_at: r.get(14)?,
    })
}

#[cfg(test)]
#[path = "tests/project_evidence.rs"]
mod tests;
