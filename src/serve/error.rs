//! Uniform HTTP error envelope for `/api/...` handlers.
//!
//! Every handler returns `Result<Json<T>, ApiError>`. The wire shape is
//! `{ "error": { "code": "...", "message": "..." } }` and the HTTP status
//! is carried on the response itself.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use crate::errors::Error;

#[derive(Debug, Serialize)]
struct Envelope<'a> {
    error: Inner<'a>,
}

#[derive(Debug, Serialize)]
struct Inner<'a> {
    code: &'a str,
    message: &'a str,
}

/// Error type returned by every handler.
#[derive(Debug)]
pub struct ApiError {
    code: &'static str,
    status: StatusCode,
    message: String,
}

impl ApiError {
    /// 404 — the requested node id is not in the graph.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: "not_found",
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    /// 400 — bad query string or body shape.
    pub fn invalid_param(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_param",
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    /// 500 — kuzu query failed.
    pub fn graph_error(message: impl Into<String>) -> Self {
        Self {
            code: "graph_error",
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }

    /// 500 — filesystem failed while loading a memory body.
    pub fn io_error(message: impl Into<String>) -> Self {
        Self {
            code: "io_error",
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        tracing::error!(target: "qwick_memory::serve", code = %self.code, message = %self.message);
        let body = Json(Envelope {
            error: Inner {
                code: self.code,
                message: &self.message,
            },
        });
        (self.status, body).into_response()
    }
}

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        match e {
            Error::Io(io) => Self::io_error(io.to_string()),
            Error::Yaml(y) => Self::graph_error(format!("yaml: {y}")),
            Error::Json(j) => Self::graph_error(format!("json: {j}")),
            Error::Toml(t) => Self::graph_error(format!("toml: {t}")),
            Error::Lance(s) => Self::graph_error(format!("lance: {s}")),
            Error::Other(s) => Self::graph_error(s),
        }
    }
}
