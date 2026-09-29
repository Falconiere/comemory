//! `project proposal list|show` / `GET /api/v1/projects/{id}/proposals[/{proposalId}]`
//! (`Verb::ProposalRead`): a keyset page of one project's proposals, newest
//! first and optionally one state, and one proposal by id — ported from the
//! platform's `listProjectProposals` and `getProjectProposal`
//! (`project-proposal-read-service.ts`). Show answers `{proposal}`; the
//! structured diff beside it is #340's. A proposal of another project is
//! `404 proposal_not_found`, the same answer as one that never existed.

use serde::{Deserialize, Serialize};

use crate::domains::projects::authority::{Actor, Command, Verb, sealed};
use crate::domains::projects::keyset;
use crate::domains::projects::limits;
use crate::domains::projects::list::vocabulary;
use crate::domains::projects::proposal_view::{self, ProposalView};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::project_proposals::{self, ProposalPage};
use crate::store::project_read;
use crate::utilities::context::Ctx;
use crate::utilities::project_error::ProjectError;
use crate::utilities::uuid;

/// The platform's proposal states.
pub const STATES: &[&str] = &[
    "pending",
    "approved",
    "changes_requested",
    "rejected",
    "superseded",
];

/// `project proposal list` request.
#[derive(Deserialize, Debug, Default, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ListRequest {
    /// The project's UUID.
    pub project_id: String,
    /// Page size, 1–100; 20 when absent.
    #[serde(default)]
    pub limit: Option<i64>,
    /// The previous page's `nextCursor`.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Only this state: pending, approved, changes_requested, rejected or superseded.
    #[serde(default)]
    pub state: Option<String>,
}

/// One page.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ListResponse {
    /// Newest first.
    pub proposals: Vec<ProposalView>,
    /// The cursor for the next page; `null` on the last one.
    pub next_cursor: Option<String>,
}

/// `project proposal show` request.
#[derive(Deserialize, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ShowRequest {
    /// The project's UUID.
    pub project_id: String,
    /// The proposal's UUID.
    pub proposal_id: String,
}

/// One proposal.
#[derive(Serialize, Debug)]
pub struct ShowResponse {
    /// Its view.
    pub proposal: ProposalView,
}

impl sealed::Sealed for ListRequest {}

impl Command for ListRequest {
    type Response = ListResponse;

    fn verb(&self) -> Verb {
        Verb::ProposalRead
    }

    /// Read one page of the project's proposals.
    fn execute(self, ctx: &mut Ctx<'_>, _actor: &Actor) -> Result<ListResponse> {
        let limit = limits::page(self.limit)?;
        let cursor = self.cursor.as_deref().map(keyset::decode).transpose()?;
        let state = vocabulary("state", self.state.as_deref(), STATES)?;
        let conn = ctx.conn()?;
        let project_id = existing(conn, &self.project_id)?;
        let mut rows = project_proposals::proposal_page(
            conn,
            &ProposalPage {
                project_id: &project_id,
                state,
                after: cursor.as_ref().map(|c| (c.at_ms, c.id.as_str())),
                limit: limit + 1,
            },
        )?;
        let has_more = rows.len() as i64 > limit;
        rows.truncate(limit as usize);
        let next_cursor = rows
            .last()
            .filter(|_| has_more)
            .map(|last| keyset::encode(last.created_at, &last.id));
        let proposals = rows
            .into_iter()
            .map(proposal_view::view)
            .collect::<Result<_>>()?;
        Ok(ListResponse {
            proposals,
            next_cursor,
        })
    }
}

impl sealed::Sealed for ShowRequest {}

impl Command for ShowRequest {
    type Response = ShowResponse;

    fn verb(&self) -> Verb {
        Verb::ProposalRead
    }

    /// Read the proposal `self.proposal_id` of project `self.project_id`.
    fn execute(self, ctx: &mut Ctx<'_>, _actor: &Actor) -> Result<ShowResponse> {
        let proposal_id = uuid::canonical(&self.proposal_id)
            .ok_or_else(|| Error::from(ProjectError::invalid_field("proposalId", "invalid")))?;
        let conn = ctx.conn()?;
        let project_id = existing(conn, &self.project_id)?;
        let row =
            project_proposals::proposal(conn, &project_id, &proposal_id)?.ok_or_else(|| {
                Error::from(ProjectError::ProposalNotFound {
                    proposal_id: self.proposal_id,
                })
            })?;
        Ok(ShowResponse {
            proposal: proposal_view::view(row)?,
        })
    }
}

/// The canonical id of the project `raw` names: `400` when malformed, `404
/// project_not_found` when no such project exists.
fn existing(conn: &Connection, raw: &str) -> Result<String> {
    let id = uuid::canonical(raw)
        .ok_or_else(|| Error::from(ProjectError::invalid_field("projectId", "invalid")))?;
    match project_read::project(conn, &id)? {
        Some(project) => Ok(project.id),
        None => Err(ProjectError::ProjectNotFound {
            project_id: raw.to_string(),
        }
        .into()),
    }
}

#[cfg(test)]
#[path = "tests/proposals.rs"]
mod tests;
