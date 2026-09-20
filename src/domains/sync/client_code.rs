//! The two platform calls behind the code-index push: read a repo's
//! manifest and post one import batch. Same base URL, credential and
//! envelope handling as [`crate::domains::sync::client`]; kept beside it so that file
//! stays under the size ceiling.

use crate::domains::sync::client::{
    http_client, managed_headers, normalize_api_url, parse_envelope,
};
use crate::domains::sync::client_protocol::validate_response;
use crate::domains::sync::exchange::{CodeImportRequest, CodeImportResponse, CodeManifestResponse};
use crate::prelude::*;
use crate::utilities::http_error::map_reqwest;

/// `GET {api}/v1/sync/code/manifest?repo=` — what the workspace holds.
///
/// # Errors
/// Transport and envelope failures.
pub fn fetch_code_manifest(
    api_url: &str,
    org_key: &str,
    repo: &str,
    revision: i64,
) -> Result<CodeManifestResponse> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/sync/code/manifest");
    let client = http_client()?;
    let resp = client
        .get(&url)
        .query(&[("repo", repo)])
        .headers(managed_headers(org_key, revision)?)
        .send()
        .map_err(map_reqwest)?;
    validate_response(&resp, revision, "code manifest")?;
    parse_envelope(resp, "code manifest")
}

/// `POST {api}/v1/sync/code/import` — one batch of a repo's projection.
///
/// # Errors
/// Transport and envelope failures.
pub fn push_code_import(
    api_url: &str,
    org_key: &str,
    body: &CodeImportRequest,
    revision: i64,
) -> Result<CodeImportResponse> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/sync/code/import");
    let client = http_client()?;
    let resp = client
        .post(&url)
        .headers(managed_headers(org_key, revision)?)
        .json(body)
        .send()
        .map_err(map_reqwest)?;
    validate_response(&resp, revision, "code import")?;
    parse_envelope(resp, "code import")
}
