//! Transport-neutral classification of a crate [`Error`]: a machine-readable
//! `code` slug plus a [`Class`] a transport maps to its own failure shape.
//! `serve::envelope::status_and_code` maps `Class` to an HTTP `StatusCode`;
//! `mcp::result::into_tool_result` maps everything but `Class::Internal` to
//! a tool-level error, `Internal` to the protocol's own error object;
//! [`exit_code`] gives `main.rs` its sysexits code, deriving a project
//! refusal's from its `Class`. One [`classify`] keeps the three adapters from
//! drifting on which error is which (Binding Rule 1).

use crate::prelude::*;
use crate::utilities::project_error::{ProjectError, RequestEdge};

/// A crate error's class, independent of any one transport's status vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// The requested resource does not exist.
    NotFound,
    /// The caller presented no valid credential.
    Unauthorized,
    /// The caller is not allowed to perform the request.
    Forbidden,
    /// The request itself is malformed.
    BadRequest,
    /// The request is well-formed but cannot be processed as given.
    Unprocessable,
    /// The request conflicts with the current state of the resource.
    Conflict,
    /// A required dependency is temporarily unavailable; retryable.
    Unavailable,
    /// A resource is locked by another writer; retryable.
    Locked,
    /// The request names a capability this build does not implement.
    NotImplemented,
    /// An unexpected failure with no more specific class.
    Internal,
}

/// The `(code, class)` pair for `e`. An exhaustive match over every [`Error`]
/// variant with no wildcard arm, so a new variant fails to compile here
/// rather than silently falling through to [`Class::Internal`]. `Io` and
/// `Sqlite` each carry a condition of their own, so their rows are decided
/// by [`classify_io`] and [`classify_sqlite`]; every other variant either
/// names its own code word or joins the shared `internal` bucket alongside
/// them (comment on that arm).
pub fn classify(e: &Error) -> (&'static str, Class) {
    match e {
        Error::Io(io) => classify_io(io),
        Error::Sqlite(_) => classify_sqlite(e),
        // No code word of its own: a broken migration chain
        // (`Error::Migration`) is a server-side schema problem the caller
        // cannot fix by retrying or rephrasing the request, the same bucket
        // `main.rs::exit_code` puts it in (EX_SOFTWARE, 70); the rest are
        // driver/parser failures with no more specific class to give.
        Error::Yaml(_) | Error::Toml(_) | Error::Git(_) | Error::Migration(_) | Error::Other(_) => {
            ("internal", Class::Internal)
        }
        Error::Json(_) => ("json", Class::BadRequest),
        Error::Ast(_) => ("ast", Class::BadRequest),
        // A database written by a NEWER comemory: the binary is older than
        // the on-disk schema. Not the caller's fault and not retryable, but
        // distinct from a broken migration — the console renders it as an
        // upgrade prompt, so it gets its own code (spec §1 `schema_mismatch`).
        Error::SchemaTooNew(_) => ("schema_mismatch", Class::Unprocessable),
        Error::VecDimMismatch { .. } => ("vec_dim_mismatch", Class::Unprocessable),
        Error::Frontmatter(_) => ("frontmatter", Class::BadRequest),
        // Two different bodies sharing one 8-hex id: the save refused to
        // overwrite the first. Conflict, not bad request — the payload is
        // fine, the store already holds that id.
        Error::IdCollision { .. } => ("id_collision", Class::Conflict),
        Error::Document(_) => ("document", Class::BadRequest),
        Error::NotFound(_) => ("not_found", Class::NotFound),
        Error::Usage(_) => ("usage", Class::BadRequest),
        Error::Config(_) => ("config", Class::BadRequest),
        Error::Forbidden(_) => ("forbidden", Class::Forbidden),
        Error::BadRequest(_) => ("bad_request", Class::BadRequest),
        Error::ConfirmationRequired(_) => ("confirmation_required", Class::BadRequest),
        Error::Unavailable(_) => ("unavailable", Class::Unavailable),
        Error::Conflict(_) => ("conflict", Class::Conflict),
        Error::EpochMismatch(_) => ("epoch_mismatch", Class::Conflict),
        Error::IndexRunning { .. } => ("index_running", Class::Conflict),
        // Only a job body ever produces `Cancelled`, and the worker turns it
        // into `JobStatus::Cancelled` before any envelope is built — listed
        // so the mapping stays total rather than falling through to
        // `Class::Internal`.
        Error::Cancelled => ("cancelled", Class::Conflict),
        Error::Unsupported(_) => ("unsupported", Class::NotImplemented),
        Error::Embedder(_) => ("embedder_unavailable", Class::Unavailable),
        // Contended `memory-save.lock`: retryable like `Embedder`, not a
        // conflict over what to do — a save and a rebuild simply took turns.
        Error::Busy(_) => ("busy", Class::Unavailable),
        // A restored engine refusing sync until its erasures are verified:
        // retryable once the operator merges the manifest, never the
        // caller's fault.
        Error::RestoreUnverified(_) => ("restore_unverified", Class::Unavailable),
        Error::Project(project) => classify_project(project),
    }
}

/// The `(code, class)` pair for a project refusal: the platform's code word
/// and the class of the HTTP status it answers there. Exhaustive like
/// [`classify`], so a new [`ProjectError`] variant is a compile error here.
pub fn classify_project(e: &ProjectError) -> (&'static str, Class) {
    match e {
        ProjectError::ProjectNotFound { .. } => ("project_not_found", Class::NotFound),
        ProjectError::WorkItemNotFound { .. } => ("work_item_not_found", Class::NotFound),
        ProjectError::ProposalNotFound { .. } => ("proposal_not_found", Class::NotFound),
        ProjectError::ExecutionNotFound { .. } => ("execution_not_found", Class::NotFound),
        ProjectError::EvidenceNotFound { .. } => ("evidence_not_found", Class::NotFound),
        // One code, two statuses: a request that never parsed is the
        // caller's malformed input (400); one that parsed but breaks a rule
        // is well-formed yet unprocessable (422).
        ProjectError::InvalidRequest { edge, .. } => match edge {
            RequestEdge::Schema => ("invalid_request", Class::BadRequest),
            RequestEdge::Invariant => ("invalid_request", Class::Unprocessable),
        },
        ProjectError::DependencyCycle { .. } => ("dependency_cycle", Class::Unprocessable),
        ProjectError::ProjectAgentScope { .. } => ("project_agent_scope", Class::Forbidden),
        ProjectError::RepoNotAllowed { .. } => ("repo_not_allowed", Class::Forbidden),
        ProjectError::ExecutionActorForbidden => ("forbidden", Class::Forbidden),
        ProjectError::ProposalStale { .. } => ("proposal_stale", Class::Conflict),
        ProjectError::ProposalAlreadyReviewed => ("proposal_already_reviewed", Class::Conflict),
        ProjectError::VersionConflict { .. } => ("version_conflict", Class::Conflict),
        ProjectError::IdempotencyConflict => ("idempotency_conflict", Class::Conflict),
        ProjectError::InvalidTransition { .. } => ("invalid_transition", Class::Conflict),
        ProjectError::DependencyBlocked { .. } => ("dependency_blocked", Class::Conflict),
        ProjectError::CompletionRequirementsUnmet { .. } => {
            ("completion_requirements_unmet", Class::Conflict)
        }
        ProjectError::EvidenceUnverified { .. } => ("evidence_unverified", Class::Conflict),
        ProjectError::ExecutionActive => ("execution_active", Class::Conflict),
        ProjectError::Unauthorized { .. } => ("unauthorized", Class::Unauthorized),
        ProjectError::ContextUnavailable { .. } => ("context_unavailable", Class::Unavailable),
        ProjectError::Invariant { .. } => ("internal_error", Class::Internal),
    }
}

/// A missing file on disk is a 404, not a 500; every other `io::Error` kind
/// (permissions, already-exists, interrupted, ...) has no more specific
/// class than [`Class::Internal`].
fn classify_io(io: &std::io::Error) -> (&'static str, Class) {
    if io.kind() == std::io::ErrorKind::NotFound {
        ("not_found", Class::NotFound)
    } else {
        ("internal", Class::Internal)
    }
}

/// SQLite's write lock held by another connection (a concurrent CLI run) is
/// transient — retry with backoff (spec §1 `store_locked`); any other
/// SQLite failure has no more specific class than [`Class::Internal`].
fn classify_sqlite(e: &Error) -> (&'static str, Class) {
    if crate::store::busy::is_locked(e) {
        ("store_locked", Class::Locked)
    } else {
        ("internal", Class::Internal)
    }
}

#[cfg(test)]
#[path = "tests/error_code.rs"]
mod tests;
