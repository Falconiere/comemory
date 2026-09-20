//! Managed sync protocol headers and response negotiation checks.

use reqwest::blocking::Response;

use crate::prelude::*;

/// Repository-policy protocol understood by this CLI.
pub const SYNC_PROTOCOL: &str = "repository-policy-v1";
/// Header carrying [`SYNC_PROTOCOL`].
pub const PROTOCOL_HEADER: &str = "x-comemory-sync-protocol";
/// Header carrying the policy revision used for a managed data request.
pub const REVISION_HEADER: &str = "x-comemory-policy-revision";

/// Refuse a managed response that does not echo the negotiated protocol and
/// exact policy revision. The body remains available to the caller on success.
pub fn validate_response(response: &Response, expected_revision: i64, context: &str) -> Result<()> {
    let protocol = response
        .headers()
        .get(PROTOCOL_HEADER)
        .and_then(|value| value.to_str().ok());
    if protocol != Some(SYNC_PROTOCOL) {
        return Err(Error::Other(format!(
            "{context}: platform does not support {SYNC_PROTOCOL}"
        )));
    }
    let revision = response
        .headers()
        .get(REVISION_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok());
    if revision != Some(expected_revision) {
        return Err(Error::Other(format!(
            "{context}: sync policy changed (expected revision {expected_revision})"
        )));
    }
    Ok(())
}
