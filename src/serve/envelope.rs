//! The `{ok, data, meta}` / `{ok, error, meta}` response envelope wrapping
//! every `/api/v1/*` response, and the one `Error → (StatusCode, code-slug)`
//! mapping ([`status_and_code`]) every HTTP error derives its status from —
//! including a failed job's `{code, message}` object
//! (`serve::jobs::JobError`) — so no surface can drift (Binding Rule 1). The
//! code-slug half of that mapping is
//! [`crate::utilities::error_code::classify`], shared with `mcp`; this file
//! keeps only the `Class → StatusCode` half.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Value, json};

use crate::prelude::*;
use crate::utilities::error_code::{self, Class};
use crate::utilities::ordered_details::OrderedDetails;

/// The single write permit (§Concurrency) is held by another mutating
/// request or job.
pub const CODE_BUSY: &str = "busy";
/// A `mutating`-flagged route was called on a `--read-only` server.
pub const CODE_READ_ONLY: &str = "read_only";
/// A confirm-gated route was called without `"confirm":true` /
/// `?confirm=true`.
pub const CODE_CONFIRMATION_REQUIRED: &str = "confirmation_required";
/// The `/api/v1/*` session token (header, query, or cookie) was missing or
/// invalid.
pub const CODE_UNAUTHORIZED: &str = "unauthorized";

/// Constructors for the `/api/v1` response envelope. Every constructor
/// returns a ready `axum::Response` — pairing the JSON body with its HTTP
/// status is exactly the "one mapping home" the design calls for, so no call
/// site re-derives a status from an `Error` on its own.
pub struct Envelope;

impl Envelope {
    /// `200 OK` success envelope: `{ok:true, data, meta:{command,elapsed_ms}}`.
    pub fn ok<T: Serialize>(command: &str, data: T, elapsed_ms: u64) -> Response {
        respond(
            StatusCode::OK,
            json!({
                "ok": true,
                "data": data,
                "meta": meta(command, elapsed_ms),
            }),
        )
    }

    /// Error envelope built from a crate [`Error`]; status and `code` come
    /// from [`status_and_code`], and the optional structured `details`
    /// member from [`error_details`] — or, for a project refusal, its own
    /// ordered details. Built by the same `error_response` every error
    /// constructor uses.
    pub fn err(command: &str, e: &Error, elapsed_ms: u64) -> Response {
        let (status, code) = status_and_code(e);
        let details = match e {
            Error::Project(project) => Some(Details::Ordered(project.details())),
            _ => error_details(e).map(Details::Plain),
        };
        error_response(command, status, code, e.to_string(), details, elapsed_ms)
    }

    /// `401`, `code:"unauthorized"` — the versioned surface's enveloped form
    /// of the router `guard`'s token check (legacy paths keep a plain-text
    /// 401; AC-11).
    pub fn unauthorized(command: &str) -> Response {
        refusal(
            command,
            StatusCode::UNAUTHORIZED,
            CODE_UNAUTHORIZED,
            "missing or invalid token",
        )
    }

    /// `503`, `code:"busy"`, `Retry-After: 5` — the single write permit
    /// (§Concurrency) is held elsewhere; a synchronous mutating request never
    /// stalls into `SQLITE_BUSY` (AC-17). Called from
    /// `serve::routes::guard_mutating` on a failed `try_acquire`.
    pub fn busy(command: &str) -> Response {
        let message = "write permit held by another request; retry shortly";
        let mut res = refusal(command, StatusCode::SERVICE_UNAVAILABLE, CODE_BUSY, message);
        res.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
        res
    }

    /// `405`, `code:"read_only"` — a `mutating` route rejected on a
    /// `--read-only` server. Called from `serve::routes::guard_mutating` and
    /// `serve::routes::guard_job`.
    pub fn read_only(command: &str) -> Response {
        refusal(
            command,
            StatusCode::METHOD_NOT_ALLOWED,
            CODE_READ_ONLY,
            "server is read-only",
        )
    }

    /// `400`, `code:"confirmation_required"` — a confirm-gated route called
    /// without `"confirm":true` (POST body) / `?confirm=true` (DELETE).
    /// `serve::routes::require_confirm` produces the same status/code by
    /// returning `Error::ConfirmationRequired` through the generic
    /// [`status_and_code`] mapping rather than calling this constructor
    /// directly; kept as a standalone builder for tests exercising the
    /// envelope shape in isolation.
    pub fn confirmation_required(command: &str) -> Response {
        let message = "this operation requires explicit confirmation";
        refusal(
            command,
            StatusCode::BAD_REQUEST,
            CODE_CONFIRMATION_REQUIRED,
            message,
        )
    }

    /// `202 Accepted`, `Location: /api/v1/jobs/{job_id}` — a job-creating
    /// `POST` route accepted the request; `data: {job_id, status:"queued"}`
    /// (§Jobs).
    pub fn accepted(command: &str, job_id: &str, elapsed_ms: u64) -> Response {
        let mut res = respond(
            StatusCode::ACCEPTED,
            json!({
                "ok": true,
                "data": {"job_id": job_id, "status": "queued"},
                "meta": meta(command, elapsed_ms),
            }),
        );
        if let Ok(loc) = HeaderValue::from_str(&format!("/api/v1/jobs/{job_id}")) {
            res.headers_mut().insert(header::LOCATION, loc);
        }
        res
    }
}

/// Map a crate [`Error`] to its `/api/v1` HTTP status and machine-readable
/// `code` slug (§Interfaces "Response envelope" table). The `code` and its
/// [`Class`] come from [`error_code::classify`] — the transport-neutral half
/// `mcp` shares — and this function's own match only ever does the one thing
/// left to a transport: pick that class's HTTP status.
pub fn status_and_code(e: &Error) -> (StatusCode, &'static str) {
    let (code, class) = error_code::classify(e);
    let status = match class {
        Class::NotFound => StatusCode::NOT_FOUND,
        Class::Unauthorized => StatusCode::UNAUTHORIZED,
        Class::Forbidden => StatusCode::FORBIDDEN,
        Class::BadRequest => StatusCode::BAD_REQUEST,
        Class::Unprocessable => StatusCode::UNPROCESSABLE_ENTITY,
        Class::Conflict => StatusCode::CONFLICT,
        Class::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        Class::Locked => StatusCode::LOCKED,
        Class::NotImplemented => StatusCode::NOT_IMPLEMENTED,
        Class::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, code)
}

/// The structured `error.details` object for the variants that carry one;
/// `None` for every other error, in which case the member is omitted
/// entirely (the error object stays `{code, message}` byte-for-byte).
pub fn error_details(e: &Error) -> Option<Value> {
    match e {
        Error::IndexRunning { repo, job_id } => Some(json!({ "repo": repo, "job_id": job_id })),
        Error::IdCollision { id } => Some(json!({ "id": id })),
        _ => None,
    }
}

/// `{ok:false, error, meta}` in declaration order.
#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    ok: bool,
    error: ErrorObject,
    meta: Meta<'a>,
}

/// `{code, message, details?}` in declaration order.
#[derive(Serialize)]
struct ErrorObject {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Details>,
}

/// A `details` member: a crate variant's `Value`, or a project refusal's
/// insertion-ordered pairs.
#[derive(Serialize)]
#[serde(untagged)]
enum Details {
    Plain(Value),
    Ordered(OrderedDetails),
}

/// `{command, elapsed_ms}` in declaration order, shared by every envelope
/// shape.
#[derive(Serialize)]
struct Meta<'a> {
    command: &'a str,
    elapsed_ms: u64,
}

/// The `meta` member for `command`.
fn meta(command: &str, elapsed_ms: u64) -> Meta<'_> {
    Meta {
        command,
        elapsed_ms,
    }
}

/// Build `{ok:false, error:{code, message, details?}, meta}` at a given
/// status. Every error constructor funnels through here, so every error body
/// serializes `ok, error{code, message, details}, meta` in that order — the
/// platform's order, which a sorted `json!` map would not keep.
/// A fixed refusal the server raises itself (no crate [`Error`], no
/// `details`, no elapsed time): the token guard, the write permit, the
/// read-only gate and the confirm gate.
fn refusal(command: &str, status: StatusCode, code: &'static str, message: &str) -> Response {
    error_response(command, status, code, message.to_string(), None, 0)
}

fn error_response(
    command: &str,
    status: StatusCode,
    code: &'static str,
    message: String,
    details: Option<Details>,
    elapsed_ms: u64,
) -> Response {
    respond(
        status,
        ErrorEnvelope {
            ok: false,
            error: ErrorObject {
                code,
                message,
                details,
            },
            meta: meta(command, elapsed_ms),
        },
    )
}

/// The one `IntoResponse` call site every envelope constructor funnels
/// through, pairing the JSON body with its HTTP status.
fn respond<T: Serialize>(status: StatusCode, body: T) -> Response {
    (status, Json(body)).into_response()
}

#[cfg(test)]
#[path = "tests/envelope.rs"]
mod tests;
