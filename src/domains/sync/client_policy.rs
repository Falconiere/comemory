//! Status negotiation for the platform's repository-managed sync protocol.

use serde::Deserialize;
use std::time::Duration;

use crate::domains::sync::client::{
    HTTP_TIMEOUT, auth_headers, http_client_with, normalize_api_url, parse_envelope,
};
use crate::prelude::*;
use crate::utilities::http_error::map_reqwest;

/// One approved GitHub repository in status data.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct ApprovedRepository {
    /// Lowercase `owner/name` policy identity.
    #[serde(rename = "fullName")]
    pub full_name: String,
    /// GitHub repository basename for display.
    pub name: String,
}

/// One administrator-confirmed legacy memory-label mapping.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct RepositoryMapping {
    /// Existing local memory label.
    pub label: String,
    /// Approved lowercase `owner/name` identity.
    #[serde(rename = "fullName")]
    pub full_name: String,
}

/// Repository policy fields returned by `GET /v1/sync/status`.
#[derive(Debug, Clone, Deserialize)]
pub struct SyncPolicyStatus {
    /// Workspace bound into the machine key.
    pub workspace_id: String,
    /// Effective approved GitHub repositories.
    pub allowlist: Vec<ApprovedRepository>,
    /// Confirmed legacy-label mappings for this workspace.
    pub repo_mappings: Vec<RepositoryMapping>,
    /// Monotonic repository-policy revision.
    pub policy_revision: i64,
    /// Managed protocol identifier.
    pub sync_protocol: String,
    /// Server-side import gate identifier.
    pub import_gate: String,
}

/// Fetch the authoritative policy before a managed sync run.
pub fn fetch(api_url: &str, org_key: &str) -> Result<SyncPolicyStatus> {
    fetch_with_timeout(api_url, org_key, HTTP_TIMEOUT)
}

/// Fetch policy under the inline push's smaller request budget.
pub fn fetch_with_timeout(
    api_url: &str,
    org_key: &str,
    timeout: Duration,
) -> Result<SyncPolicyStatus> {
    let base = normalize_api_url(api_url);
    let response = http_client_with(timeout)?
        .get(format!("{base}/v1/sync/status"))
        .headers(auth_headers(org_key)?)
        .send()
        .map_err(map_reqwest)?;
    parse_envelope(response, "sync policy")
}
