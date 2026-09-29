//! `project proposal submit` / `POST /api/v1/projects/{id}/proposals` /
//! MCP `project_propose` (`Verb::ProposalCreate`): an immutable plan
//! proposal, ported from the platform's `submitProjectProposal`
//! (`project-proposal-service.ts`). One immediate transaction, under the
//! command's receipt (#327), writes the `pending` row, moves a `draft`
//! project to `planning` (its `version` bumped), and records one
//! `project.proposal_submitted` event; any refusal writes none of it.
//!
//! Request-level rules run before the store opens; the project's state and
//! the stale-base check run inside the transaction, so they see the state
//! the row is written against. The live-plan preflight is #337's.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::domains::projects::activity::{self, Event};
use crate::domains::projects::authority::{Actor, Command, Verb, sealed};
use crate::domains::projects::limits;
use crate::domains::projects::operation_rules;
use crate::domains::projects::operations::{Operation, bounded};
use crate::domains::projects::proposal_view::{self, ProposalView};
use crate::domains::projects::receipt::{self, Applied, Keyed, Ran};
use crate::domains::projects::timestamp::now_ms;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::project_proposals::{self, NewProposal};
use crate::store::project_read::{self, ProjectRow};
use crate::utilities::activity::command;
use crate::utilities::context::Ctx;
use crate::utilities::project_error::ProjectError;
use crate::utilities::uuid;

/// The receipt's and the row's command type.
const COMMAND_TYPE: &str = "project.proposal.submit";
/// The platform's `PROJECT_RATIONALE_MAX`: a rationale and each risk.
pub const RATIONALE_MAX: usize = 4000;
/// Assumptions or risks per proposal.
pub const NOTES_MAX_COUNT: usize = 50;

/// `project proposal submit` request: the platform's body plus the project.
#[derive(Deserialize, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Request {
    /// The project's UUID (the HTTP path's `{id}`).
    pub project_id: String,
    /// The platform's workspace id: accepted and ignored.
    #[serde(default)]
    pub workspace_id: Option<Value>,
    /// The caller's retry key, 1–200 UTF-16 units, scoped to the principal.
    pub idempotency_key: String,
    /// The plan version the operations were written against.
    pub base_plan_version: i64,
    /// 1–200 typed plan operations, at most 256 KiB serialized.
    #[serde(deserialize_with = "bounded")]
    #[schemars(with = "Vec<Operation>")]
    pub operations: Vec<Operation>,
    /// Why the change is proposed, 1–4000 characters.
    pub rationale: String,
    /// Up to 50 assumptions, each 1–500 characters.
    #[serde(default)]
    pub assumptions: Vec<String>,
    /// Up to 50 risks, each 1–4000 characters.
    #[serde(default)]
    pub risks: Vec<String>,
}

/// The stored proposal.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Response {
    /// Its view.
    pub proposal: ProposalView,
}

/// The platform's `project.proposal_submitted` payload, keys in its order.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SubmittedPayload {
    base_plan_version: i64,
    operation_count: usize,
    risk_count: usize,
}

impl sealed::Sealed for Request {}

impl Command for Request {
    type Response = Response;

    fn verb(&self) -> Verb {
        Verb::ProposalCreate
    }

    /// Submit as `actor`, or replay its receipt, then record the telemetry row.
    fn execute(self, ctx: &mut Ctx<'_>, actor: &Actor) -> Result<Response> {
        let started = Instant::now();
        let ran = submit(ctx, actor, self);
        receipt::record(
            ctx,
            command::PROJECT_PROPOSAL_SUBMIT,
            started,
            ran,
            |r| json!({"id": r.proposal.id, "projectId": r.proposal.project_id}),
        )
    }
}

impl Request {
    /// The platform's `digestBody`: every field but the key, as received.
    fn digest_body(&self) -> Result<Value> {
        Ok(json!({
            "basePlanVersion": self.base_plan_version,
            "operations": serde_json::to_value(&self.operations)?,
            "rationale": self.rationale,
            "assumptions": self.assumptions,
            "risks": self.risks,
        }))
    }

    /// Every rule a request meets without the store, normalizing the
    /// operations in place.
    fn validate(&mut self) -> Result<String> {
        let id = uuid::canonical(&self.project_id)
            .ok_or_else(|| Error::from(ProjectError::invalid_field("projectId", "invalid")))?;
        if self.base_plan_version < 0 {
            return Err(ProjectError::over_limit("basePlanVersion", "too_small", 0).into());
        }
        operation_rules::validate(&mut self.operations)?;
        limits::text("rationale", &self.rationale, 1, RATIONALE_MAX)?;
        let assumptions = &self.assumptions;
        limits::list(
            "assumptions",
            assumptions,
            NOTES_MAX_COUNT,
            limits::CONSTRAINT_MAX,
        )?;
        limits::list("risks", &self.risks, NOTES_MAX_COUNT, RATIONALE_MAX)?;
        Ok(id)
    }
}

/// Check the key, validate, then write under the receipt in one immediate
/// transaction.
fn submit(ctx: &mut Ctx<'_>, actor: &Actor, mut req: Request) -> Result<Ran<Response>> {
    let keyed = Keyed::new(&req.idempotency_key, COMMAND_TYPE, &req.digest_body()?)?;
    let project_id = req.validate()?;
    let digest = keyed.digest().to_string();
    receipt::run(ctx.conn()?, actor, &keyed, |tx| {
        let response = write(tx, actor, &project_id, &req, &digest)?;
        Ok(Applied {
            response,
            project_id: project_id.clone(),
        })
    })
}

/// Refuse a project that cannot take this proposal, then write the row,
/// the first-proposal status move and the event, and answer the row as
/// stored.
fn write(
    tx: &Connection,
    actor: &Actor,
    project_id: &str,
    req: &Request,
    digest: &str,
) -> Result<Response> {
    let project = project_read::project(tx, project_id)?.ok_or_else(|| {
        Error::from(ProjectError::ProjectNotFound {
            project_id: req.project_id.clone(),
        })
    })?;
    accepts(&project, req.base_plan_version)?;
    // Stamped once the writer lock is held, so commit order and `created_at`
    // order agree for every writer of this database.
    let at_ms = now_ms();
    let id = insert(tx, actor, project_id, req, digest, at_ms)?;
    project_proposals::start_planning(tx, project_id, at_ms)?;
    let payload = SubmittedPayload {
        base_plan_version: req.base_plan_version,
        operation_count: req.operations.len(),
        risk_count: req.risks.len(),
    };
    let event = Event {
        project_id,
        event_type: "project.proposal_submitted",
        entity_type: "proposal",
        entity_id: &id,
        payload: &payload,
    };
    activity::record(tx, actor, &event, at_ms)?;
    let row = project_proposals::proposal(tx, project_id, &id)?.ok_or_else(|| {
        Error::from(ProjectError::Invariant {
            invariant: "project_proposal_row".to_string(),
            message: format!("proposal {id} vanished inside its own creation transaction"),
        })
    })?;
    Ok(Response {
        proposal: proposal_view::view(row)?,
    })
}

/// The immutable `pending` row, under a minted id.
fn insert(
    tx: &Connection,
    actor: &Actor,
    project_id: &str,
    req: &Request,
    digest: &str,
    at_ms: i64,
) -> Result<String> {
    let proposer = actor.principal();
    let id = uuid::new_v4()?;
    project_proposals::insert(
        tx,
        &NewProposal {
            id: &id,
            project_id,
            base_plan_version: req.base_plan_version,
            operations: &serde_json::to_string(&req.operations)?,
            assumptions: &serde_json::to_string(&req.assumptions)?,
            risks: &serde_json::to_string(&req.risks)?,
            rationale: &req.rationale,
            proposer_type: proposer.principal_type.as_str(),
            proposer_id: &proposer.id,
            request_digest: digest,
            at_ms,
        },
    )?;
    Ok(id)
}

/// The platform's `assertProjectAcceptsProposals`, then its stale-base check.
fn accepts(project: &ProjectRow, base_plan_version: i64) -> Result<()> {
    let refused = |reason: &str| {
        let reason = reason.to_string();
        Err(ProjectError::InvalidTransition { reason }.into())
    };
    if project.archived_at.is_some() {
        return refused("This project is archived");
    }
    if matches!(project.status.as_str(), "completed" | "canceled") {
        return refused("This project is closed to new proposals");
    }
    if project.current_plan_version != base_plan_version {
        return Err(ProjectError::ProposalStale {
            base_plan_version,
            current_plan_version: project.current_plan_version,
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/propose.rs"]
mod tests;
