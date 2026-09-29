#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! The project error contract (#322) through every adapter that reads it:
//! each of the twenty-two codes asserted as its `classify` code, the real
//! `Envelope::err` HTTP status and body, the CLI exit code and the MCP
//! channel; `invalid_request`'s two statuses; the platform's own bytes for
//! three sample codes (`tests/fixtures/platform-project-errors.txt`,
//! generated from comemory.io's `project-errors.ts`); and `project_not_found`
//! never becoming an existence probe.

use axum::body::to_bytes;
use axum::http::StatusCode;
use comemory::errors::Error;
use comemory::mcp::result::into_tool_result;
use comemory::serve::envelope::Envelope;
use comemory::utilities::error_code::classify;
use comemory::utilities::exit_code::exit_code;
use comemory::utilities::ordered_details::OrderedDetails;
use comemory::utilities::project_error::{ProjectError, RequestEdge};
use serde_json::Value;

/// The platform's `/v1` bodies, one `<code> <status> <body>` line each.
const PLATFORM_FIXTURE: &str = include_str!("fixtures/platform-project-errors.txt");

/// Which MCP failure channel a code travels on.
#[derive(Debug, PartialEq)]
enum Mcp {
    /// A tool-level `isError` result carrying `{code, message}`.
    Tool,
    /// A protocol error carrying only the code word.
    Protocol,
}

/// The real response bytes `Envelope::err` produces for `e`.
async fn envelope_bytes(e: &Error) -> (StatusCode, String) {
    let response = Envelope::err("test", e, 0);
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// The MCP channel `into_tool_result` puts `e` on, checking it carries `code`.
fn mcp_channel(e: Error, code: &str) -> Mcp {
    let outcome: comemory::errors::Result<Value> = Err(e);
    match into_tool_result(outcome) {
        Ok(result) => {
            assert_eq!(result.is_error, Some(true), "{code}: tool error flag");
            let body = result.structured_content.expect("structured content");
            assert_eq!(body["code"], code, "{code}: MCP code");
            Mcp::Tool
        }
        Err(protocol) => {
            assert_eq!(protocol.message, code, "{code}: protocol message");
            Mcp::Protocol
        }
    }
}

/// An invariant-edge `invalid_request`, as preflight raises it.
fn invariant_request() -> ProjectError {
    ProjectError::InvalidRequest {
        edge: RequestEdge::Invariant,
        message: "operations exceeds the per-proposal cap".into(),
        details: OrderedDetails::from_pairs(vec![
            ("field", Value::from("operations")),
            ("reason", Value::from("cap_exceeded")),
        ]),
    }
}

/// One constructed value per code, `invalid_request` at both edges.
fn rows() -> Vec<(ProjectError, &'static str, u16, i32, Mcp)> {
    let s = String::from;
    vec![
        (
            ProjectError::ProjectNotFound {
                project_id: s("prj_1"),
            },
            "project_not_found",
            404,
            64,
            Mcp::Tool,
        ),
        (
            ProjectError::WorkItemNotFound {
                work_item_id: s("wi_1"),
            },
            "work_item_not_found",
            404,
            64,
            Mcp::Tool,
        ),
        (
            ProjectError::ProposalNotFound {
                proposal_id: s("pp_1"),
            },
            "proposal_not_found",
            404,
            64,
            Mcp::Tool,
        ),
        (
            ProjectError::ExecutionNotFound {
                execution_id: s("ex_1"),
            },
            "execution_not_found",
            404,
            64,
            Mcp::Tool,
        ),
        (
            ProjectError::EvidenceNotFound {
                evidence_id: s("ev_1"),
            },
            "evidence_not_found",
            404,
            64,
            Mcp::Tool,
        ),
        (
            ProjectError::invalid_field("cursor", "invalid"),
            "invalid_request",
            400,
            64,
            Mcp::Tool,
        ),
        (invariant_request(), "invalid_request", 422, 65, Mcp::Tool),
        (
            ProjectError::DependencyCycle {
                work_item_ids: vec![s("wi_1"), s("wi_2")],
            },
            "dependency_cycle",
            422,
            65,
            Mcp::Tool,
        ),
        (
            ProjectError::ProjectAgentScope {
                reason: s("capability not granted"),
            },
            "project_agent_scope",
            403,
            70,
            Mcp::Tool,
        ),
        (
            ProjectError::RepoNotAllowed {
                repo: s("acme/api"),
            },
            "repo_not_allowed",
            403,
            70,
            Mcp::Tool,
        ),
        (
            ProjectError::ExecutionActorForbidden,
            "forbidden",
            403,
            70,
            Mcp::Tool,
        ),
        // Adjacent to its sibling: `dedup` below only drops neighbours.
        (
            ProjectError::TierForbidden {
                reason: s("Only the project lead or a workspace admin may run this command"),
            },
            "forbidden",
            403,
            70,
            Mcp::Tool,
        ),
        (
            ProjectError::ProposalStale {
                base_plan_version: 3,
                current_plan_version: 5,
            },
            "proposal_stale",
            409,
            75,
            Mcp::Tool,
        ),
        (
            ProjectError::ProposalAlreadyReviewed,
            "proposal_already_reviewed",
            409,
            75,
            Mcp::Tool,
        ),
        (
            ProjectError::VersionConflict { current_version: 7 },
            "version_conflict",
            409,
            75,
            Mcp::Tool,
        ),
        (
            ProjectError::IdempotencyConflict,
            "idempotency_conflict",
            409,
            75,
            Mcp::Tool,
        ),
        (
            ProjectError::InvalidTransition {
                reason: s("the project is archived"),
            },
            "invalid_transition",
            409,
            75,
            Mcp::Tool,
        ),
        (
            ProjectError::DependencyBlocked {
                blocker_work_item_ids: vec![s("wi_9")],
            },
            "dependency_blocked",
            409,
            75,
            Mcp::Tool,
        ),
        (
            ProjectError::CompletionRequirementsUnmet {
                unmet_criterion_ids: vec![s("crit_a")],
                unverified_criterion_ids: vec![],
                work_item_ids: vec![],
            },
            "completion_requirements_unmet",
            409,
            75,
            Mcp::Tool,
        ),
        (
            ProjectError::EvidenceUnverified {
                criterion_ids: vec![s("crit_b")],
            },
            "evidence_unverified",
            409,
            75,
            Mcp::Tool,
        ),
        (
            ProjectError::ExecutionActive,
            "execution_active",
            409,
            75,
            Mcp::Tool,
        ),
        (
            ProjectError::Unauthorized {
                reason: s("Missing or invalid principal stamp"),
            },
            "unauthorized",
            401,
            70,
            Mcp::Tool,
        ),
        (
            ProjectError::ContextUnavailable {
                reason: s("engine timed out"),
            },
            "context_unavailable",
            503,
            69,
            Mcp::Tool,
        ),
        (
            ProjectError::Invariant {
                invariant: s("plan_operation_kind"),
                message: s("unknown plan operation"),
            },
            "internal_error",
            500,
            70,
            Mcp::Protocol,
        ),
    ]
}

/// Every code reads the same code word across `classify`, the HTTP
/// envelope, the CLI exit and MCP — 24 rows for 22 codes, because
/// `invalid_request` answers at two edges and `forbidden` is raised both for
/// another actor's execution and for a human below a verb's tier (#315).
#[tokio::test]
async fn every_project_code_reads_the_same_across_adapters() {
    let rows = rows();
    assert_eq!(rows.len(), 24);
    let mut codes: Vec<&str> = rows.iter().map(|row| row.1).collect();
    codes.dedup();
    assert_eq!(codes.len(), 22, "twenty-two distinct codes");

    for (project, code, status, exit, mcp) in rows {
        let err = Error::Project(project);
        assert_eq!(classify(&err).0, code, "classify code");
        let (http, body) = envelope_bytes(&err).await;
        assert_eq!(http.as_u16(), status, "{code}: HTTP status");
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["ok"], false, "{code}: ok");
        assert_eq!(body["error"]["code"], code, "{code}: HTTP code");
        assert_eq!(exit_code(&err), exit, "{code}: CLI exit");
        assert_eq!(mcp_channel(err, code), mcp, "{code}: MCP channel");
    }
}

/// `invalid_request` is one code with two statuses, and the edge decides:
/// a request that never parsed (schema, malformed cursor) is the caller's
/// malformed input, `400`; one that parsed but breaks an entity or cap rule
/// is well-formed but unprocessable, `422`. The platform draws the line in
/// the same place (oRPC `BAD_REQUEST` vs preflight).
#[tokio::test]
async fn invalid_request_status_follows_the_edge_not_the_code() {
    let schema = Error::Project(ProjectError::invalid_field("cursor", "invalid"));
    let invariant = Error::Project(invariant_request());
    assert_eq!(classify(&schema).0, classify(&invariant).0);
    assert_eq!(envelope_bytes(&schema).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        envelope_bytes(&invariant).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

/// The engine's `error` member is the platform's, byte for byte, for the
/// fixture lines — including `proposal_stale`, whose `details` keys are not
/// alphabetical, and the four authority refusals #315 ports (a member below
/// the lead tier, a lead below the owner tier, an agent on a human-only verb,
/// an agent missing a capability).
#[tokio::test]
async fn project_error_bodies_equal_the_platform_fixture() {
    let s = String::from;
    let samples = [
        ProjectError::ProjectNotFound {
            project_id: s("prj_01J9Z3K5V7"),
        },
        ProjectError::ProposalStale {
            base_plan_version: 3,
            current_plan_version: 5,
        },
        ProjectError::CompletionRequirementsUnmet {
            unmet_criterion_ids: vec![s("crit_a")],
            unverified_criterion_ids: vec![s("crit_b")],
            work_item_ids: vec![s("wi_1")],
        },
        ProjectError::TierForbidden {
            reason: s("Only the project lead or a workspace admin may run this command"),
        },
        ProjectError::TierForbidden {
            reason: s("Only a workspace owner or admin can delete a project"),
        },
        ProjectError::ProjectAgentScope {
            reason: s("This command requires a signed-in human"),
        },
        ProjectError::ProjectAgentScope {
            reason: s("This grant does not carry the health.update capability"),
        },
    ];
    let lines: Vec<&str> = PLATFORM_FIXTURE
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    assert_eq!(lines.len(), samples.len());

    for (project, line) in samples.into_iter().zip(lines) {
        let mut parts = line.splitn(3, ' ');
        let (code, status, platform) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        let err = Error::Project(project);
        assert_eq!(classify(&err).0, code);
        let error_member = platform
            .strip_prefix(r#"{"error":"#)
            .and_then(|rest| rest.strip_suffix('}'))
            .expect("platform body is {\"error\":{...}}");
        let expected = format!(
            r#"{{"ok":false,"error":{error_member},"meta":{{"command":"test","elapsed_ms":0}}}}"#
        );
        let (http, body) = envelope_bytes(&err).await;
        assert_eq!(http.as_u16().to_string(), status, "{code}: status");
        assert_eq!(body, expected, "{code}: body bytes");
    }
}

/// An unknown project and one outside the caller's scope are the same
/// value — `ProjectNotFound` has no field to say why — so their bodies are
/// byte-identical and name only the fixed message and the supplied id.
#[tokio::test]
async fn project_not_found_is_not_an_existence_probe() {
    let unknown = Error::Project(ProjectError::ProjectNotFound {
        project_id: "prj_42".into(),
    });
    let out_of_scope = Error::Project(ProjectError::ProjectNotFound {
        project_id: "prj_42".into(),
    });
    let (status_a, body_a) = envelope_bytes(&unknown).await;
    let (status_b, body_b) = envelope_bytes(&out_of_scope).await;
    assert_eq!((status_a, &body_a), (status_b, &body_b));
    assert_eq!(
        body_a,
        r#"{"ok":false,"error":{"code":"project_not_found","message":"Project not found","details":{"code":"project_not_found","projectId":"prj_42"}},"meta":{"command":"test","elapsed_ms":0}}"#
    );
}
