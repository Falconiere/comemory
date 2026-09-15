//! The two platform calls behind the code-index push: read a repo's
//! manifest and post one import batch. Same base URL, credential and
//! envelope handling as [`crate::sync::client`]; kept beside it so that file
//! stays under the size ceiling.

use crate::api::sync::{CodeImportRequest, CodeImportResponse, CodeManifestResponse};
use crate::http_error::map_reqwest;
use crate::prelude::*;
use crate::sync::client::{auth_headers, http_client, normalize_api_url, parse_envelope};

/// `GET {api}/v1/sync/code/manifest?repo=` — what the workspace holds.
///
/// # Errors
/// Transport and envelope failures.
pub fn fetch_code_manifest(
    api_url: &str,
    org_key: &str,
    repo: &str,
) -> Result<CodeManifestResponse> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/sync/code/manifest");
    let client = http_client()?;
    let resp = client
        .get(&url)
        .query(&[("repo", repo)])
        .headers(auth_headers(org_key)?)
        .send()
        .map_err(map_reqwest)?;
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
) -> Result<CodeImportResponse> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/sync/code/import");
    let client = http_client()?;
    let resp = client
        .post(&url)
        .headers(auth_headers(org_key)?)
        .json(body)
        .send()
        .map_err(map_reqwest)?;
    parse_envelope(resp, "code import")
}
