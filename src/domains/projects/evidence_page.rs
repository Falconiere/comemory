//! `project evidence list` / `GET /api/v1/projects/{id}/evidence` (#346): a
//! keyset page of one project's evidence over `(created_at, id)`, newest
//! first, filtered by kind, trust and work item alone or together — the
//! platform's `listProjectEvidence`. `pending` and `invalid` rows are listed
//! like any other: they are audit records, and what they cannot do is
//! satisfy a criterion (#349). An unknown `workItemId` filter, or a cursor
//! from another project, selects nothing, as on the platform; an unknown
//! project is `404 project_not_found`, as every engine read answers it.

use serde::Serialize;

use crate::domains::projects::activity_page::split;
use crate::domains::projects::authority::{Actor, Command, Verb, sealed};
use crate::domains::projects::evidence::{self, EvidenceView};
use crate::domains::projects::evidence_check::canonical_id;
use crate::domains::projects::keyset::{self, Positioned};
use crate::domains::projects::limits;
use crate::domains::projects::list::vocabulary;
use crate::prelude::*;
use crate::store::project_evidence::{self, EvidencePage, EvidenceRow};
use crate::store::project_read;
use crate::utilities::context::Ctx;
use crate::utilities::project_error::ProjectError;

/// Rows per page when the caller names no limit (the platform's
/// `EVIDENCE_PAGE_DEFAULT`).
const DEFAULT_LIMIT: i64 = 50;

/// `project evidence list` / `GET /api/v1/projects/{id}/evidence` request.
#[derive(serde::Deserialize, Debug, Default, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Request {
    /// The project's UUID.
    pub project_id: String,
    /// Page size, 1–100; 50 when absent.
    #[serde(default)]
    pub limit: Option<i64>,
    /// The previous page's `nextCursor`.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Only this kind.
    #[serde(default)]
    pub kind: Option<String>,
    /// Only this trust: `verified`, `self_reported`, `pending` or `invalid`.
    #[serde(default)]
    pub trust: Option<String>,
    /// Only evidence on this work item.
    #[serde(default)]
    pub work_item_id: Option<String>,
}

/// One page of a project's evidence, newest first.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    /// The page's rows.
    pub evidence: Vec<EvidenceView>,
    /// The cursor for the next page; `null` on the last one.
    pub next_cursor: Option<String>,
}

impl sealed::Sealed for Request {}

impl Command for Request {
    type Response = Response;

    fn verb(&self) -> Verb {
        Verb::EvidenceRead
    }

    /// Read one page of `self.project_id`'s evidence.
    fn execute(self, ctx: &mut Ctx<'_>, _actor: &Actor) -> Result<Response> {
        let project_id = canonical_id("projectId", &self.project_id)?;
        let limit = limits::page_within(self.limit, DEFAULT_LIMIT, limits::PAGE_MAX)?;
        let cursor = self.cursor.as_deref().map(keyset::decode).transpose()?;
        let kind = vocabulary("kind", self.kind.as_deref(), evidence::KINDS)?;
        let trust = vocabulary("trust", self.trust.as_deref(), evidence::TRUSTS)?;
        let work_item_id = self
            .work_item_id
            .as_deref()
            .map(|id| canonical_id("workItemId", id))
            .transpose()?;
        let conn = ctx.conn()?;
        if project_read::project(conn, &project_id)?.is_none() {
            let project_id = self.project_id;
            return Err(ProjectError::ProjectNotFound { project_id }.into());
        }
        let rows = project_evidence::page(
            conn,
            &EvidencePage {
                project_id: &project_id,
                kind,
                trust: trust.map(evidence::trust_filter),
                work_item_id: work_item_id.as_deref(),
                before: cursor.as_ref().map(|c| (c.at_ms, c.id.as_str())),
                limit: limit + 1,
            },
        )?;
        let (rows, next_cursor) = split(rows, limit);
        Ok(Response {
            evidence: rows
                .into_iter()
                .map(evidence::view)
                .collect::<Result<_>>()?,
            next_cursor,
        })
    }
}

impl Positioned for EvidenceRow {
    fn position(&self) -> (i64, &str) {
        (self.created_at, &self.id)
    }
}

#[cfg(test)]
#[path = "tests/evidence_page.rs"]
mod tests;
