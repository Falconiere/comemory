//! The project refusal vocabulary: every condition a project command answers
//! with, carried by the crate error as [`crate::errors::Error::Project`].
//!
//! **Twenty-two codes.** The platform's
//! `apps/api/src/services/projects/project-errors.ts` raises twenty-one and
//! its principal resolution (`project-principal.ts`) adds `unauthorized`.
//! The epic's first count said eighteen: it missed `execution_not_found`,
//! `evidence_not_found`, `forbidden` and `internal_error`. The grant-scoped
//! `not_found` and the key-scope subcodes (`device_key_scope`,
//! `org_key_scope`) stay on the platform and are not ported.
//!
//! Each variant's `Display` is the platform's `message` and [`details`] its
//! `details` object, key for key and in the platform's order, so the console
//! parses an engine refusal without change. The code word and its class are
//! decided once, in [`crate::utilities::error_code::classify`].
//!
//! [`details`]: ProjectError::details

use serde_json::Value;
use thiserror::Error;

use crate::utilities::error_code::classify_project;
use crate::utilities::ordered_details::OrderedDetails;

/// Which edge refused an `invalid_request`: one code, two statuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestEdge {
    /// The request never parsed — a schema rejection or a malformed cursor
    /// (the platform's oRPC `BAD_REQUEST`). Answers `400`.
    Schema,
    /// The request parsed but violates an entity, cap or cross-reference
    /// invariant (the platform's preflight). Answers `422`.
    Invariant,
}

/// A project refusal. Variant order follows the HTTP status it answers.
#[derive(Debug, Error)]
pub enum ProjectError {
    /// `404` — one answer for an unknown project and one outside the
    /// caller's reach. Only the caller-supplied id is carried, so no call
    /// site can encode *why*, and the two stay byte-identical: never an
    /// existence probe.
    #[error("Project not found")]
    ProjectNotFound {
        /// The id the caller asked for.
        project_id: String,
    },
    /// `404` — an unknown, excluded or inaccessible work item.
    #[error("Work item not found")]
    WorkItemNotFound {
        /// The id the caller asked for.
        work_item_id: String,
    },
    /// `404` — an unknown proposal, or one of another project.
    #[error("Proposal not found")]
    ProposalNotFound {
        /// The id the caller asked for.
        proposal_id: String,
    },
    /// `404` — an unknown or foreign execution.
    #[error("Execution not found")]
    ExecutionNotFound {
        /// The id the caller asked for.
        execution_id: String,
    },
    /// `404` — an unknown or foreign evidence row.
    #[error("Evidence not found")]
    EvidenceNotFound {
        /// The id the caller asked for.
        evidence_id: String,
    },
    /// `400` or `422` by [`RequestEdge`]; `details` are the caller's
    /// (`field, reason` at the schema edge).
    #[error("{message}")]
    InvalidRequest {
        /// Which edge refused it, and therefore its status.
        edge: RequestEdge,
        /// A fixed, content-free message.
        message: String,
        /// The structured details, in wire order.
        details: OrderedDetails,
    },
    /// `422` — the post-proposal `blocks` graph has a cycle.
    #[error("The proposed dependency graph contains a cycle")]
    DependencyCycle {
        /// The items involved, in cycle order.
        work_item_ids: Vec<String>,
    },
    /// `403` — a project-agent credential asked outside its grant.
    #[error("{reason}")]
    ProjectAgentScope {
        /// A short fixed string naming which limit refused it.
        reason: String,
    },
    /// `403` — a repository outside the workspace allowlist.
    #[error("This repository is not on the workspace's GitHub App allowlist")]
    RepoNotAllowed {
        /// The repository the caller named.
        repo: String,
    },
    /// `403 forbidden` — acting on another actor's execution.
    #[error("Only this execution's own actor may run this command")]
    ExecutionActorForbidden,
    /// `409` — the proposal's base plan version is no longer current.
    #[error("The plan has changed since this proposal was written")]
    ProposalStale {
        /// The plan version the proposal was written against.
        base_plan_version: i64,
        /// The project's current plan version.
        current_plan_version: i64,
    },
    /// `409` — the proposal is no longer `pending`.
    #[error("This proposal has already been reviewed")]
    ProposalAlreadyReviewed,
    /// `409` — `expectedVersion` no longer matches the stored row.
    #[error("The project has changed since it was last read")]
    VersionConflict {
        /// The stored version, so a caller can re-read and retry.
        current_version: i64,
    },
    /// `409` — an idempotency key reused for a different command or body.
    #[error("This idempotency key was already used for a different command")]
    IdempotencyConflict,
    /// `409` — a status or archive edge the lifecycle does not allow.
    #[error("{reason}")]
    InvalidTransition {
        /// Which edge was refused.
        reason: String,
    },
    /// `409` — an unfinished `blocks` dependency prevents the transition.
    #[error("An unfinished blocker prevents this transition")]
    DependencyBlocked {
        /// Every still-unfinished blocker.
        blocker_work_item_ids: Vec<String>,
    },
    /// `409` — completion ran with required criteria or items unmet.
    #[error("This project still has unmet completion requirements")]
    CompletionRequirementsUnmet {
        /// Criteria with no acceptable linked evidence at all.
        unmet_criterion_ids: Vec<String>,
        /// `verified`-policy criteria holding only `self_reported` evidence.
        unverified_criterion_ids: Vec<String>,
        /// Required work items still incomplete.
        work_item_ids: Vec<String>,
    },
    /// `409` — a `verified` criterion has only `self_reported` evidence.
    #[error("Required evidence exists but is not verified")]
    EvidenceUnverified {
        /// Every criterion in that state.
        criterion_ids: Vec<String>,
    },
    /// `409` — a non-stale execution already owns the work item.
    #[error("A non-stale execution already owns this work item")]
    ExecutionActive,
    /// `401` — hosted mode: the principal stamp is missing or invalid.
    #[error("{reason}")]
    Unauthorized {
        /// Which credential check failed.
        reason: String,
    },
    /// `503` — the engine context for a work packet was unavailable.
    #[error("{reason}")]
    ContextUnavailable {
        /// The underlying failure, never a memory or source body.
        reason: String,
    },
    /// `500 internal_error` — a guard the project code owns was reached.
    #[error("{message}")]
    Invariant {
        /// Which guard fired.
        invariant: String,
        /// What it found.
        message: String,
    },
}

impl ProjectError {
    /// A schema-edge `invalid_request` (`400`) for `field`, worded exactly as
    /// the platform's `invalidRequestFrom`: `"<field> is <reason>"`.
    pub fn invalid_field(field: &str, reason: &str) -> Self {
        Self::InvalidRequest {
            edge: RequestEdge::Schema,
            message: format!("{field} is {reason}"),
            details: OrderedDetails::from_pairs(vec![
                ("field", Value::from(field)),
                ("reason", Value::from(reason)),
            ]),
        }
    }

    /// The `details` object, in the platform's key order. Every code but
    /// `invalid_request` leads with `code`; `invalid_request` carries the
    /// caller's pairs unchanged.
    pub fn details(&self) -> OrderedDetails {
        if let Self::InvalidRequest { details, .. } = self {
            return details.clone();
        }
        OrderedDetails::coded(classify_project(self).0, self.detail_pairs())
    }

    /// The members after `code`, per variant; empty for a bare `{code}`.
    fn detail_pairs(&self) -> Vec<(&'static str, Value)> {
        match self {
            Self::ProjectNotFound { project_id } => vec![("projectId", id(project_id))],
            Self::WorkItemNotFound { work_item_id } => vec![("workItemId", id(work_item_id))],
            Self::ProposalNotFound { proposal_id } => vec![("proposalId", id(proposal_id))],
            Self::ExecutionNotFound { execution_id } => vec![("executionId", id(execution_id))],
            Self::EvidenceNotFound { evidence_id } => vec![("evidenceId", id(evidence_id))],
            Self::DependencyCycle { work_item_ids } => vec![("workItemIds", ids(work_item_ids))],
            Self::RepoNotAllowed { repo } => vec![("repo", id(repo))],
            Self::ProposalStale {
                base_plan_version: base,
                current_plan_version: current,
            } => vec![
                ("basePlanVersion", Value::from(*base)),
                ("currentPlanVersion", Value::from(*current)),
            ],
            Self::VersionConflict { current_version } => {
                vec![("currentVersion", Value::from(*current_version))]
            }
            Self::DependencyBlocked {
                blocker_work_item_ids: blockers,
            } => vec![("blockerWorkItemIds", ids(blockers))],
            Self::CompletionRequirementsUnmet {
                unmet_criterion_ids: unmet,
                unverified_criterion_ids: unverified,
                work_item_ids,
            } => vec![
                ("criterionIds", ids(&[&unmet[..], unverified].concat())),
                ("unmetCriterionIds", ids(unmet)),
                ("unverifiedCriterionIds", ids(unverified)),
                ("workItemIds", ids(work_item_ids)),
            ],
            Self::EvidenceUnverified { criterion_ids } => {
                vec![("criterionIds", ids(criterion_ids))]
            }
            Self::Invariant { invariant, .. } => vec![("invariant", id(invariant))],
            Self::InvalidRequest { .. }
            | Self::ProjectAgentScope { .. }
            | Self::ExecutionActorForbidden
            | Self::ProposalAlreadyReviewed
            | Self::IdempotencyConflict
            | Self::InvalidTransition { .. }
            | Self::ExecutionActive
            | Self::Unauthorized { .. }
            | Self::ContextUnavailable { .. } => Vec::new(),
        }
    }
}

/// One string as a JSON value.
fn id(value: &str) -> Value {
    Value::from(value)
}

/// A string list as a JSON array.
fn ids(values: &[String]) -> Value {
    Value::from(values.to_vec())
}

#[cfg(test)]
#[path = "tests/project_error.rs"]
mod tests;
