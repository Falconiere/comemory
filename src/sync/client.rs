//! Blocking HTTP client for the comemory platform API.
//!
//! Wire assumptions (Better Auth device flow + v1 OpenAPI / sync surface):
//! - `POST {api}/auth/device/code` with `{client_id:"comemory-cli"}` → device/user codes.
//! - `POST {api}/auth/device/token` with OAuth device-code grant → session access token.
//! - `POST {api}/v1/device/mint-device-key` with `{deviceName}` + Bearer session → `cmk_` secret.
//! - Authenticated calls use `Authorization: Bearer {cmk_…}` and `X-Comemory-Workspace: {id}`.
//! - `GET {api}/v1/workspaces` lists workspaces as `{ workspaces: [{ workspace: {id,name} }] }`.
//! - Allowlist lives on `GET {api}/v1/sync/status` (`data.allowlist`, `data.allowlist_etag`).
//! - Sync routes wrap payloads in `{ok,data,meta}` (Worker status + engine forward).

use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde::Serialize;

use crate::api::sync::{ChangesResponse, ImportRequest, ImportResponse, ManifestResponse};
use crate::prelude::*;
use crate::sync::match_key::AllowlistRepo;

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

/// Device key mint response from `POST /v1/device/mint-device-key`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct MintDeviceKeyResponse {
    /// Full device API secret (`cmk_…`).
    pub secret: String,
    /// Display prefix of the minted key.
    #[serde(rename = "keyPrefix")]
    pub key_prefix: String,
    /// Personal workspace id returned at login.
    #[serde(rename = "personalWorkspaceId")]
    pub personal_workspace_id: String,
    /// Platform API base URL (may differ from the login URL).
    #[serde(rename = "apiUrl")]
    pub api_url: String,
    /// User email when the platform returns one.
    #[serde(default)]
    pub email: Option<String>,
}

/// One workspace row for CLI display.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkspaceRow {
    /// Workspace id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Whether this is the user's personal workspace (filled by the CLI).
    #[serde(default)]
    pub personal: bool,
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

/// `GET /v1/sync/status` data payload (Worker-only).
#[derive(Debug, Deserialize)]
struct SyncStatusData {
    #[serde(default)]
    allowlist: Vec<AllowlistRepo>,
    #[serde(default)]
    allowlist_etag: Option<String>,
}

/// Nested workspace view from `GET /v1/workspaces`.
#[derive(Debug, Deserialize)]
struct WorkspaceListBody {
    #[serde(default)]
    workspaces: Vec<WorkspaceViewWire>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceViewWire {
    workspace: WorkspaceInner,
}

#[derive(Debug, Deserialize)]
struct WorkspaceInner {
    id: String,
    name: String,
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

/// Exchange a session bearer token for a long-lived device key.
pub fn mint_device_key(
    api_url: &str,
    session_token: &str,
    device_name: &str,
) -> Result<MintDeviceKeyResponse> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/device/mint-device-key");
    let client = http_client()?;
    let resp = client
        .post(&url)
        .header(AUTHORIZATION, bearer_value(session_token)?)
        .json(&serde_json::json!({ "deviceName": device_name }))
        .send()
        .map_err(map_reqwest)?;
    parse_json(resp, "mint device key")
}

/// List workspaces visible to the device key.
pub fn list_workspaces(api_url: &str, device_key: &str) -> Result<Vec<WorkspaceRow>> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/workspaces");
    let client = http_client()?;
    let resp = client
        .get(&url)
        .headers(auth_headers(device_key, None)?)
        .send()
        .map_err(map_reqwest)?;
    let body: WorkspaceListBody = parse_json(resp, "list workspaces")?;
    Ok(body
        .workspaces
        .into_iter()
        .map(|row| WorkspaceRow {
            id: row.workspace.id,
            name: row.workspace.name,
            personal: false,
        })
        .collect())
}

/// Fetch the org-repo allowlist from `GET /v1/sync/status`.
///
/// Personal / unbound workspaces yield an empty list. When `etag` matches the
/// server's `allowlist_etag`, returns that etag with an empty repo vec so the
/// caller can keep its cache.
pub fn fetch_allowlist(
    api_url: &str,
    device_key: &str,
    workspace_id: &str,
    etag: Option<&str>,
) -> Result<(Vec<AllowlistRepo>, Option<String>)> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/sync/status");
    let client = http_client()?;
    let resp = client
        .get(&url)
        .headers(auth_headers(device_key, Some(workspace_id))?)
        .send()
        .map_err(map_reqwest)?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok((Vec::new(), None));
    }
    let body: SyncStatusData = parse_envelope(resp, "fetch sync status")?;
    let new_etag = body.allowlist_etag;
    if let (Some(prev), Some(next)) = (etag, new_etag.as_deref())
        && prev == next
    {
        return Ok((Vec::new(), new_etag));
    }
    Ok((body.allowlist, new_etag))
}

/// Pull sync log entries above `since` from the platform.
pub fn pull_changes(
    api_url: &str,
    device_key: &str,
    workspace_id: &str,
    since: i64,
    limit: usize,
) -> Result<ChangesResponse> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/sync/changes");
    let client = http_client()?;
    let resp = client
        .get(&url)
        .query(&[("since", since.to_string()), ("limit", limit.to_string())])
        .headers(auth_headers(device_key, Some(workspace_id))?)
        .send()
        .map_err(map_reqwest)?;
    parse_envelope(resp, "pull changes")
}

/// Push a batch of local changes to the platform.
pub fn push_import(
    api_url: &str,
    device_key: &str,
    workspace_id: &str,
    body: &ImportRequest,
) -> Result<ImportResponse> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/sync/import");
    let client = http_client()?;
    let resp = client
        .post(&url)
        .headers(auth_headers(device_key, Some(workspace_id))?)
        .json(body)
        .send()
        .map_err(map_reqwest)?;
    parse_envelope(resp, "push import")
}

/// Fetch the remote content-hash manifest for verify/repair.
pub fn fetch_manifest(
    api_url: &str,
    device_key: &str,
    workspace_id: &str,
) -> Result<ManifestResponse> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/sync/manifest");
    let client = http_client()?;
    let resp = client
        .get(&url)
        .headers(auth_headers(device_key, Some(workspace_id))?)
        .send()
        .map_err(map_reqwest)?;
    parse_envelope(resp, "fetch manifest")
}

fn bearer_value(token: &str) -> Result<HeaderValue> {
    HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|e| Error::Other(format!("authorization header: {e}")))
}

fn auth_headers(device_key: &str, workspace_id: Option<&str>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, bearer_value(device_key)?);
    if let Some(ws) = workspace_id {
        headers.insert(
            "x-comemory-workspace",
            HeaderValue::from_str(ws)
                .map_err(|e| Error::Other(format!("workspace header: {e}")))?,
        );
    }
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
