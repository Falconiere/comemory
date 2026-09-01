//! The `{ok, data, meta}` / `{ok, error, meta}` response envelope wrapping
//! every `/api/v1/*` response, and the one `Error → (StatusCode, code-slug)`
//! mapping ([`status_and_code`]) every HTTP error derives its status from —
//! including a failed job's `{code, message}` object
//! (`serve::jobs::JobError`) — so no surface can drift (Binding Rule 1).

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Value, json};

use crate::prelude::*;

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
    /// member from [`error_details`].
    pub fn err(command: &str, e: &Error, elapsed_ms: u64) -> Response {
        let (status, code) = status_and_code(e);
        let mut error = json!({ "code": code, "message": e.to_string() });
        if let (Some(details), Some(obj)) = (error_details(e), error.as_object_mut()) {
            obj.insert("details".into(), details);
        }
        respond(
            status,
            json!({
                "ok": false,
                "error": error,
                "meta": meta(command, elapsed_ms),
            }),
        )
    }

    /// `401`, `code:"unauthorized"` — the versioned surface's enveloped form
    /// of the router `guard`'s token check (legacy paths keep a plain-text
    /// 401; AC-11).
    pub fn unauthorized(command: &str) -> Response {
        error_response(
            command,
            StatusCode::UNAUTHORIZED,
            CODE_UNAUTHORIZED,
            "missing or invalid token".to_string(),
            0,
        )
    }

    /// `503`, `code:"busy"`, `Retry-After: 5` — the single write permit
    /// (§Concurrency) is held elsewhere; a synchronous mutating request never
    /// stalls into `SQLITE_BUSY` (AC-17). Called from
    /// `serve::routes::guard_mutating` on a failed `try_acquire`.
    pub fn busy(command: &str) -> Response {
        let mut res = error_response(
            command,
            StatusCode::SERVICE_UNAVAILABLE,
            CODE_BUSY,
            "write permit held by another request; retry shortly".to_string(),
            0,
        );
        res.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
        res
    }

    /// `405`, `code:"read_only"` — a `mutating` route rejected on a
    /// `--read-only` server. Called from `serve::routes::guard_mutating` and
    /// `serve::routes::guard_job`.
    pub fn read_only(command: &str) -> Response {
        error_response(
            command,
            StatusCode::METHOD_NOT_ALLOWED,
            CODE_READ_ONLY,
            "server is read-only".to_string(),
            0,
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
        error_response(
            command,
            StatusCode::BAD_REQUEST,
            CODE_CONFIRMATION_REQUIRED,
            "this operation requires explicit confirmation".to_string(),
            0,
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
/// `code` slug (§Interfaces "Response envelope" table).
pub fn status_and_code(e: &Error) -> (StatusCode, &'static str) {
    match e {
        Error::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
        Error::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
        Error::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
        Error::ConfirmationRequired(_) => (StatusCode::BAD_REQUEST, CODE_CONFIRMATION_REQUIRED),
        Error::Usage(_) => (StatusCode::BAD_REQUEST, "usage"),
        Error::Config(_) => (StatusCode::BAD_REQUEST, "config"),
        Error::Frontmatter(_) => (StatusCode::BAD_REQUEST, "frontmatter"),
        Error::Document(_) => (StatusCode::BAD_REQUEST, "document"),
        Error::Ast(_) => (StatusCode::BAD_REQUEST, "ast"),
        Error::Json(_) => (StatusCode::BAD_REQUEST, "json"),
        Error::VecDimMismatch { .. } => (StatusCode::UNPROCESSABLE_ENTITY, "vec_dim_mismatch"),
        Error::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        Error::Embedder(_) => (StatusCode::SERVICE_UNAVAILABLE, "embedder_unavailable"),
        Error::IndexRunning { .. } => (StatusCode::CONFLICT, "index_running"),
        // Only a job body ever produces `Cancelled`, and the worker turns it
        // into `JobStatus::Cancelled` before any envelope is built — listed
        // so the mapping stays total rather than falling through to 500.
        Error::Cancelled => (StatusCode::CONFLICT, "cancelled"),
        Error::Unsupported(_) => (StatusCode::NOT_IMPLEMENTED, "unsupported"),
        // A database written by a NEWER comemory: the binary is older than
        // the on-disk schema. Not the caller's fault and not retryable, but
        // distinct from a broken migration — the console renders it as an
        // upgrade prompt, so it gets its own code (spec §1 `schema_mismatch`).
        Error::SchemaTooNew(_) => (StatusCode::UNPROCESSABLE_ENTITY, "schema_mismatch"),
        // SQLite's write lock is held by another connection (a concurrent
        // CLI run): transient, retry with backoff (spec §1 `store_locked`).
        Error::Sqlite(e) if sqlite_is_locked(e) => (StatusCode::LOCKED, "store_locked"),
        // A missing file on disk is a 404, not a 500.
        Error::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
            (StatusCode::NOT_FOUND, "not_found")
        }
        // Everything else — including a broken migration chain
        // (`Error::Migration`), a server-side schema problem the caller
        // cannot fix by retrying or rephrasing the request, the same
        // bucket `main.rs::exit_code` puts it in (EX_SOFTWARE, 70).
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

/// The structured `error.details` object for the variants that carry one;
/// `None` for every other error, in which case the member is omitted
/// entirely (the error object stays `{code, message}` byte-for-byte).
pub fn error_details(e: &Error) -> Option<Value> {
    match e {
        Error::IndexRunning { repo, job_id } => Some(json!({ "repo": repo, "job_id": job_id })),
        _ => None,
    }
}

/// Whether a `rusqlite` error is SQLite's `SQLITE_BUSY` / `SQLITE_LOCKED`
/// — the write lock is held elsewhere and the statement can be retried.
fn sqlite_is_locked(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(ffi, _)
            if matches!(
                ffi.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

/// `{command, elapsed_ms}` shared by every envelope shape.
fn meta(command: &str, elapsed_ms: u64) -> Value {
    json!({ "command": command, "elapsed_ms": elapsed_ms })
}

/// Build `{ok:false, error:{code, message}, meta}` at a given status.
fn error_response(
    command: &str,
    status: StatusCode,
    code: &'static str,
    message: String,
    elapsed_ms: u64,
) -> Response {
    respond(
        status,
        json!({
            "ok": false,
            "error": { "code": code, "message": message },
            "meta": meta(command, elapsed_ms),
        }),
    )
}

/// The one `IntoResponse` call site every envelope constructor funnels
/// through, pairing the JSON body with its HTTP status.
fn respond(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

#[cfg(test)]
#[path = "tests/envelope.rs"]
mod tests;
