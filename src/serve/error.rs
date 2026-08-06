//! Map the crate `Error` to an HTTP response for the legacy `comemory serve`
//! API (unversioned routes; bare plain-text bodies, no envelope).
//!
//! Handlers return `Result<T, ApiError>` so the `?` operator threads crate
//! errors straight out; `ApiError` is a thin newtype that owns the plain-text
//! body. The HTTP status itself is *not* re-derived here — it delegates to
//! [`crate::serve::envelope::status_and_code`], the one `Error → StatusCode`
//! mapping both the legacy and `/api/v1` surfaces share (Binding Rule 1).
//! Keeping the `axum` dependency here (rather than in `src/errors.rs`)
//! preserves the CLI's error enum as a pure, framework-free type.

use axum::response::{IntoResponse, Response};

use crate::errors::Error;
use crate::serve::envelope::status_and_code;

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
        let (status, _code) = status_and_code(&self.0);
        let message = match &self.0 {
            Error::NotFound(m) => m.clone(),
            Error::Forbidden(m) => m.clone(),
            Error::BadRequest(m) => m.clone(),
            // A missing file on disk is a 404, not a 500.
            Error::Io(e) if e.kind() == std::io::ErrorKind::NotFound => e.to_string(),
            other => other.to_string(),
        };
        (status, message).into_response()
    }
}
