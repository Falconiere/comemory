//! `project evidence add` / `POST /api/v1/projects/{id}/evidence` (#346):
//! one typed evidence row against a project or one of its work items, ported
//! from the platform's `createProjectEvidence` outside an execution. The
//! request is checked in the platform's field order, and its metadata encoded
//! under the 16 KiB cap, before any store access;
//! then one immediate transaction, under the command's receipt (#327),
//! checks the project, the repository gate, the criteria and the work item,
//! writes the row, its criterion links and one `project.evidence.recorded`
//! event, and reads the row back. Nothing is verified here: the claim's
//! initial trust is [`Claim::initial`]'s (#348 resolves `pending` rows).

use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::domains::projects::activity::{self, Event};
use crate::domains::projects::authority::{Actor, Command, Verb, sealed};
use crate::domains::projects::evidence::{self, Claim, EvidenceView, Initial, StoredMetadata};
use crate::domains::projects::evidence_check::{self, Valid};
use crate::domains::projects::local_only::LocalOnly;
use crate::domains::projects::receipt::{self, Applied, Keyed, Ran};
use crate::domains::projects::timestamp::now_ms;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::project_evidence::{self, NewEvidence};
use crate::store::project_read;
use crate::utilities::activity::command;
use crate::utilities::context::Ctx;
use crate::utilities::project_error::{ProjectError, RequestEdge};
use crate::utilities::uuid;

/// `project evidence add` / `POST /api/v1/projects/{id}/evidence` request:
/// the platform's body plus the project id its path carries.
#[derive(Deserialize, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Request {
    /// The project's UUID.
    pub project_id: String,
    /// The caller's retry key, 1–200 UTF-16 units, scoped to the principal.
    pub idempotency_key: String,
    /// The work item it supports; the project itself when absent.
    #[serde(default)]
    pub work_item_id: Option<String>,
    /// `commit`, `pull_request`, `test_run`, `deployment`, `session`,
    /// `decision`, `memory` or `external_url`.
    pub kind: String,
    /// The system it came from, 1–120 characters.
    pub source: String,
    /// Its id in that system, 1–256 characters.
    #[serde(default)]
    pub external_id: Option<String>,
    /// An absolute URL, at most 2048 characters.
    #[serde(default)]
    pub url: Option<String>,
    /// Canonical `owner/name`; must be one of the project's repositories.
    #[serde(default)]
    pub repo: Option<String>,
    /// Hex commit id, 1–256 characters.
    #[serde(default)]
    pub commit_sha: Option<String>,
    /// A JSON object stored with the claim; 16 KiB encoded with it.
    #[serde(default)]
    pub metadata: Option<Map<String, Value>>,
    /// Up to 20 of this project's criterion ids the evidence may satisfy.
    #[serde(default)]
    pub criterion_ids: Vec<String>,
}

/// The recorded evidence.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Response {
    /// Its view.
    pub evidence: EvidenceView,
    /// `local_only` when the project is bound by a transfer (#342): the
    /// change stays in this data directory. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<LocalOnly>,
}

/// The platform's `project.evidence.recorded` payload, keys in its order.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordedPayload<'a> {
    kind: &'a str,
    trust: &'a str,
    reason: Option<&'a str>,
    work_item_id: Option<&'a str>,
    execution_id: Option<&'a str>,
    criterion_ids: &'a [String],
}

impl sealed::Sealed for Request {}

impl Command for Request {
    type Response = Response;

    fn verb(&self) -> Verb {
        Verb::EvidenceCreate
    }

    /// Record the evidence as `actor`, or replay its receipt, then record
    /// the telemetry row.
    fn execute(self, ctx: &mut Ctx<'_>, actor: &Actor) -> Result<Response> {
        let started = Instant::now();
        let ran = add(ctx, actor, self);
        receipt::record(
            ctx,
            command::PROJECT_EVIDENCE_ADD,
            started,
            ran,
            |r| json!({"id": r.evidence.id, "kind": r.evidence.kind, "trust": r.evidence.trust}),
        )
    }
}

impl Request {
    /// The digested body: every field as received except `idempotencyKey`.
    /// `projectId` counts, because one principal may reach many projects:
    /// the same key on another project is another command.
    fn digest_body(&self) -> Value {
        json!({
            "projectId": self.project_id,
            "workItemId": self.work_item_id,
            "executionId": null,
            "kind": self.kind,
            "source": self.source,
            "externalId": self.external_id,
            "url": self.url,
            "repo": self.repo,
            "commitSha": self.commit_sha,
            "metadata": self.metadata.clone().unwrap_or_default(),
            "criterionIds": self.criterion_ids,
        })
    }
}

/// Check the key and the request, then write under the receipt.
fn add(ctx: &mut Ctx<'_>, actor: &Actor, req: Request) -> Result<Ran<Response>> {
    let keyed = Keyed::new(
        &req.idempotency_key,
        "project.evidence.create",
        &req.digest_body(),
    )?;
    let valid = evidence_check::validate(req)?;
    let claim = Claim {
        kind: &valid.kind,
        repo: valid.repo.as_deref(),
        commit_sha: valid.commit_sha.as_deref(),
        external_id: valid.external_id.as_deref(),
    };
    claim.check_shape()?;
    let initial = claim.initial();
    // Encoded before the store opens, so an over-cap body is a `422` for
    // any project, known or not.
    let metadata = StoredMetadata {
        repo: valid.repo.as_deref(),
        commit_sha: valid.commit_sha.as_deref(),
        reason: initial.reason,
        claim: &valid.metadata,
        provider: None,
    }
    .encode()?;
    let decided = Decided { initial, metadata };
    receipt::run(ctx.conn()?, actor, &keyed, |tx| {
        let response = write(tx, actor, &valid, &decided)?;
        Ok(Applied {
            response,
            project_id: valid.project_id.clone(),
        })
    })
}

/// What the attach decided before the store opened: the claim's initial
/// trust and its encoded, capped metadata.
struct Decided {
    initial: Initial,
    metadata: String,
}

/// The checks that read the store, then the row, its links and its event.
fn write(tx: &Connection, actor: &Actor, v: &Valid, decided: &Decided) -> Result<Response> {
    let initial = decided.initial;
    let pid = v.project_id.as_str();
    in_scope(tx, v)?;
    let linked = distinct(&v.criterion_ids);
    let members = project_evidence::members(tx, pid, v.work_item_id.as_deref(), &linked)?;
    criteria_in_project(&linked, &members.criteria)?;
    if v.work_item_id.is_some() && !members.work_item {
        let work_item_id = v.raw_work_item_id.clone().unwrap_or_default();
        return Err(ProjectError::WorkItemNotFound { work_item_id }.into());
    }
    let id = uuid::new_v4()?;
    let at_ms = now_ms();
    insert(tx, actor, v, decided, &id, at_ms)?;
    project_evidence::link_criteria(tx, &id, &linked)?;
    let payload = RecordedPayload {
        kind: &v.kind,
        trust: initial.trust,
        reason: initial.reason,
        work_item_id: v.work_item_id.as_deref(),
        execution_id: None,
        criterion_ids: &v.criterion_ids,
    };
    let event = Event {
        project_id: pid,
        event_type: "project.evidence.recorded",
        entity_type: "evidence",
        entity_id: &id,
        payload: &payload,
    };
    let recorded = activity::record(tx, actor, &event, at_ms)?;
    recorded_view(tx, pid, &id, recorded.local_only)
}

/// `404 project_not_found` for an unknown project, then `403
/// repo_not_allowed` unless the claim's `repo`, when it names one, is one of
/// the project's own.
fn in_scope(tx: &Connection, v: &Valid) -> Result<()> {
    if project_read::project(tx, &v.project_id)?.is_none() {
        let project_id = v.raw_project_id.clone();
        return Err(ProjectError::ProjectNotFound { project_id }.into());
    }
    let Some(repo) = &v.repo else {
        return Ok(());
    };
    let owned = project_read::relations(tx, std::slice::from_ref(&v.project_id))?;
    if owned.repositories.iter().any(|(_, r)| r == repo) {
        return Ok(());
    }
    let repo = repo.clone();
    Err(ProjectError::RepoNotAllowed { repo }.into())
}

/// `422 invalid_request` naming every id that is not a criterion of this
/// project: a caller that believes it linked a completion gate must not be
/// told it succeeded.
fn criteria_in_project(ids: &[String], found: &[String]) -> Result<()> {
    let unknown: Vec<Value> = ids
        .iter()
        .filter(|id| !found.contains(id))
        .map(|id| Value::from(id.as_str()))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let message = "One or more criteria do not belong to this project";
    let mut refusal = ProjectError::invalid_field("criterionIds", "unknown")
        .at(RequestEdge::Invariant, Some(message));
    if let ProjectError::InvalidRequest { details, .. } = &mut refusal {
        details.push("criterionIds", Value::from(unknown));
    }
    Err(refusal.into())
}

/// `ids` without repeats, in first-seen order.
fn distinct(ids: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(ids.len());
    for id in ids {
        if !out.contains(id) {
            out.push(id.clone());
        }
    }
    out
}

/// The `project_evidence` row itself.
fn insert(
    tx: &Connection,
    actor: &Actor,
    v: &Valid,
    decided: &Decided,
    id: &str,
    at_ms: i64,
) -> Result<()> {
    let creator = actor.principal();
    project_evidence::insert(
        tx,
        &NewEvidence {
            id,
            project_id: &v.project_id,
            work_item_id: v.work_item_id.as_deref(),
            kind: &v.kind,
            source: &v.source,
            external_id: v.external_id.as_deref(),
            url: v.url.as_deref(),
            trust: decided.initial.trust,
            metadata: &decided.metadata,
            creator_type: creator.principal_type.as_str(),
            creator_id: &creator.id,
            at_ms,
        },
    )
}

/// The view of the row this transaction just wrote.
fn recorded_view(
    tx: &Connection,
    project_id: &str,
    id: &str,
    local_only: Option<LocalOnly>,
) -> Result<Response> {
    let row = project_evidence::one(tx, project_id, id)?.ok_or_else(|| {
        Error::from(ProjectError::Invariant {
            invariant: "evidence_row_missing".to_string(),
            message: format!("evidence {id} vanished inside its own attach transaction"),
        })
    })?;
    Ok(Response {
        evidence: evidence::view(row)?,
        warnings: local_only.into_iter().collect(),
    })
}

#[cfg(test)]
#[path = "tests/evidence_add.rs"]
mod tests;
