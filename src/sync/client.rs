//! Blocking HTTP client for the comemory platform API.
//!
//! Wire assumptions (Better Auth device flow + v1 OpenAPI / sync surface):
//! - `POST {api}/auth/device/code` with `{client_id:"comemory-cli"}` → device/user codes.
//! - `POST {api}/auth/device/token` with OAuth device-code grant → session access token.
//! - `POST {api}/v1/device/mint-org-key` + Bearer session → an org-scoped `cmk_` secret.
//! - Authenticated calls send `Authorization: Bearer {cmk_…}` and **nothing else**:
//!   the key is scoped to one organization, so the platform derives the
//!   workspace from it. No call names a workspace.
//! - Sync routes wrap payloads in `{ok,data,meta}` (Worker status + engine forward).

use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde::Deserialize;

use crate::api::sync::{ChangesResponse, ImportRequest, ImportResponse, ManifestResponse};
use crate::prelude::*;

const CLIENT_ID: &str = "comemory-cli";
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Device authorization response from `POST /auth/device/code`.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCodeResponse {
    /// Opaque device code for polling.
    pub device_code: String,
    /// Human-entered user code.
    pub user_code: String,
    /// Browser URL for verification.
    pub verification_uri: String,
    /// Seconds until the device code expires.
    #[serde(default)]
    pub expires_in: u64,
    /// Suggested poll interval in seconds.
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_interval() -> u64 {
    5
}

/// Token poll response — pending errors use HTTP 400 with `authorization_pending`.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    /// Bearer access token (session) before minting a device key.
    #[serde(default)]
    pub access_token: Option<String>,
    /// Error slug when authorization is still pending or failed.
    #[serde(default)]
    pub error: Option<String>,
}

/// Platform `{ok,data,meta}` / `{ok,error,meta}` envelope.
#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> {
    ok: bool,
    data: Option<T>,
    #[serde(default)]
    error: Option<ApiErrorBody>,
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

/// Build a shared blocking client with a fixed timeout.
fn http_client() -> Result<Client> {
    Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| Error::Other(format!("http client: {e}")))
}

/// Normalize a platform base URL (trim trailing `/`).
fn normalize_api_url(api_url: &str) -> String {
    api_url.trim_end_matches('/').to_string()
}

/// Start the OAuth device authorization flow.
pub fn device_code(api_url: &str) -> Result<DeviceCodeResponse> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/auth/device/code");
    let client = http_client()?;
    let resp = client
        .post(&url)
        .json(&serde_json::json!({ "client_id": CLIENT_ID }))
        .send()
        .map_err(map_reqwest)?;
    parse_json(resp, "device code")
}

/// Poll for an access token after the user approves the device code.
pub fn poll_token(api_url: &str, device_code: &str) -> Result<TokenResponse> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/auth/device/token");
    let client = http_client()?;
    let resp = client
        .post(&url)
        .json(&serde_json::json!({
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            "device_code": device_code,
            "client_id": CLIENT_ID,
        }))
        .send()
        .map_err(map_reqwest)?;
    parse_json(resp, "device token")
}

/// Pull sync log entries above `since` from the platform.
pub fn pull_changes(
    api_url: &str,
    org_key: &str,
    since: i64,
    limit: usize,
) -> Result<ChangesResponse> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/sync/changes");
    let client = http_client()?;
    let resp = client
        .get(&url)
        .query(&[("since", since.to_string()), ("limit", limit.to_string())])
        .headers(auth_headers(org_key)?)
        .send()
        .map_err(map_reqwest)?;
    parse_envelope(resp, "pull changes")
}

/// Push a batch of local changes to the platform.
pub fn push_import(api_url: &str, org_key: &str, body: &ImportRequest) -> Result<ImportResponse> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/sync/import");
    let client = http_client()?;
    let resp = client
        .post(&url)
        .headers(auth_headers(org_key)?)
        .json(body)
        .send()
        .map_err(map_reqwest)?;
    parse_envelope(resp, "push import")
}

/// Fetch the remote content-hash manifest for verify/repair.
pub fn fetch_manifest(api_url: &str, org_key: &str) -> Result<ManifestResponse> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/sync/manifest");
    let client = http_client()?;
    let resp = client
        .get(&url)
        .headers(auth_headers(org_key)?)
        .send()
        .map_err(map_reqwest)?;
    parse_envelope(resp, "fetch manifest")
}

fn bearer_value(token: &str) -> Result<HeaderValue> {
    HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|e| Error::Other(format!("authorization header: {e}")))
}

/// Authorization only. The org-scoped key already names the workspace, so
/// sending `X-Comemory-Workspace` would let a caller ask for one the key
/// cannot reach — the header is gone rather than ignored.
fn auth_headers(org_key: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, bearer_value(org_key)?);
    Ok(headers)
}

fn map_reqwest(e: reqwest::Error) -> Error {
    Error::Other(format!("http: {e}"))
}

fn parse_json<T: for<'de> Deserialize<'de>>(
    resp: reqwest::blocking::Response,
    ctx: &str,
) -> Result<T> {
    let status = resp.status();
    let text = resp.text().map_err(map_reqwest)?;
    if !status.is_success() {
        return Err(Error::Other(format!("{ctx}: HTTP {status}: {text}")));
    }
    serde_json::from_str(&text).map_err(|e| Error::Other(format!("{ctx}: json: {e}; body: {text}")))
}

fn parse_envelope<T: for<'de> Deserialize<'de>>(
    resp: reqwest::blocking::Response,
    ctx: &str,
) -> Result<T> {
    let status = resp.status();
    let text = resp.text().map_err(map_reqwest)?;
    if !status.is_success() {
        return Err(Error::Other(format!("{ctx}: HTTP {status}: {text}")));
    }
    let env: ApiEnvelope<T> = serde_json::from_str(&text)
        .map_err(|e| Error::Other(format!("{ctx}: envelope json: {e}; body: {text}")))?;
    if !env.ok {
        let err = env.error.unwrap_or_else(|| ApiErrorBody {
            code: "unknown".into(),
            message: text.clone(),
        });
        return Err(Error::Other(format!(
            "{ctx}: {} — {}",
            err.code, err.message
        )));
    }
    env.data
        .ok_or_else(|| Error::Other(format!("{ctx}: envelope missing data; body: {text}")))
}

#[cfg(test)]
#[path = "tests/client.rs"]
mod tests;
