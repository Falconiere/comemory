//! Map the crate `Error` to an HTTP response for the `comemory serve` API.
//!
//! Handlers return `Result<T, ApiError>` so the `?` operator threads crate
//! errors straight out; `ApiError` is a thin newtype that owns the single
//! `Error` → `StatusCode` mapping. Keeping the `axum` dependency here (rather
//! than in `src/errors.rs`) preserves the CLI's error enum as a pure,
//! framework-free type.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::errors::Error;

/// Newtype wrapper letting `crate::errors::Error` cross the axum handler
/// boundary as an HTTP response. Construct via `From<Error>` (so `?` works).
pub struct ApiError(pub Error);

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self.0 {
            Error::NotFound(m) => (StatusCode::NOT_FOUND, m.clone()),
            Error::Forbidden(m) => (StatusCode::FORBIDDEN, m.clone()),
            Error::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            // A missing file on disk is a 404, not a 500.
            Error::Io(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (StatusCode::NOT_FOUND, e.to_string())
            }
            other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
        };
        (status, message).into_response()
    }
}
