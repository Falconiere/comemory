//! Map outbound HTTP client errors into [`crate::Error`].
//!
//! Shared by `domains::sync::client` and `capture::client` so the reqwest source-chain
//! enrichment cannot drift between the two platform callers (#138).

use crate::prelude::*;

/// Format a reqwest failure as `http: {e}: {source}…` for [`Error::Other`].
pub(crate) fn map_reqwest(e: reqwest::Error) -> Error {
    let mut msg = format!("http: {e}");
    let mut src = std::error::Error::source(&e);
    while let Some(cause) = src {
        msg.push_str(": ");
        msg.push_str(&cause.to_string());
        src = std::error::Error::source(cause);
    }
    Error::Other(msg)
}
