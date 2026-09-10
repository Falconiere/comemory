//! RFC 8628 device authorization + device-key mint against the platform.
//!
//! Flow: `POST /auth/device/code` → print user code / URI → poll
//! `POST /auth/device/token` → `POST /v1/device/mint-device-key` with
//! `Authorization: Bearer <access_token>`. The minted key is the unbound
//! device key `sync` uses; the workspace comes from `X-Comemory-Workspace`
//! per call, never from the key.

use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::Serialize;

use crate::cloud::CLIENT_ID;
use crate::fetch::{self, Request};
use crate::prelude::*;
use crate::sync::auth_file::AuthFile;

const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const DEFAULT_INTERVAL_SECS: u64 = 5;

/// Successful device-code grant (RFC 8628 §3.2).
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCodeResponse {
    /// Opaque device code the CLI polls with.
    pub device_code: String,
    /// Short code the human types at the verification URI.
    pub user_code: String,
    /// Console page that accepts the user code.
    pub verification_uri: String,
    /// Prefill URI when the server supplies one.
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    /// Lifetime of the device code in seconds.
    pub expires_in: u64,
    /// Minimum poll interval in seconds.
    #[serde(default)]
    pub interval: Option<u64>,
}

/// One-shot mint payload from `POST /v1/device/mint-device-key`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MintResponse {
    secret: String,
    key_prefix: String,
    personal_workspace_id: String,
    api_url: String,
    #[serde(default)]
    email: Option<String>,
}

/// `GET /v1/workspaces` rows (fields we surface).
#[derive(Debug, Clone, Deserialize)]
struct WorkspaceList {
    workspaces: Vec<WorkspaceListRow>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkspaceListRow {
    workspace: WorkspaceInfo,
}

/// One workspace row from `GET /v1/workspaces`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkspaceInfo {
    /// Workspace UUID.
    pub id: String,
    /// Display name.
    pub name: String,
}

/// Outcome of a completed [`login`].
#[derive(Debug, Clone, Serialize)]
pub struct LoginOutcome {
    /// Device credentials written to `auth.json` (includes the secret once).
    pub credentials: AuthFile,
}

/// `comemory auth status` report — never reprints the full secret.
#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    /// Whether a usable secret was found (file and/or `COMEMORY_API_KEY`).
    pub authenticated: bool,
    /// API base in use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    /// Personal workspace id echoed from `auth.json` when the key still works.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Personal workspace display name from the live `GET /v1/workspaces`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    /// Key prefix for display (never the full secret).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_prefix: Option<String>,
}

/// Run the full device login against `api_url` for `device_name`, printing
/// the user code to `progress` (typically stderr). Returns minted device
/// credentials (caller persists).
pub fn login(api_url: &str, device_name: &str, progress: &mut impl Write) -> Result<LoginOutcome> {
    let code = request_device_code(api_url)?;
    let uri = code
        .verification_uri_complete
        .as_deref()
        .unwrap_or(code.verification_uri.as_str());
    writeln!(progress, "Visit {uri}\nand enter code: {}", code.user_code)?;
    let _ = progress.flush();
    let access_token = poll_access_token(api_url, &code)?;
    let minted = mint_device_key(api_url, &access_token, device_name)?;
    // The platform echoes its own canonical base; an empty one means the
    // deployment did not set `BETTER_AUTH_URL`, so keep the URL we dialed.
    let resolved_api_url = if minted.api_url.trim().is_empty() {
        api_url.to_string()
    } else {
        minted.api_url
    };
    Ok(LoginOutcome {
        credentials: AuthFile {
            secret: minted.secret,
            key_prefix: minted.key_prefix,
            personal_workspace_id: minted.personal_workspace_id,
            api_url: resolved_api_url,
            device_name: device_name.to_string(),
            email: minted.email,
        },
    })
}

/// Probe whether `secret` authenticates against `api_url`. `personal_id` is
/// the workspace recorded at login, used to name the row in the report;
/// device keys are unbound, so `GET /v1/workspaces` is the surface they may
/// call (`/v1/workspaces/current` answers `403 device_key_scope`).
pub fn workspace_status(
    api_url: &str,
    secret: &str,
    personal_id: Option<&str>,
) -> Result<StatusReport> {
    let Some(rows) = list_workspaces(api_url, secret)? else {
        return Ok(StatusReport {
            authenticated: false,
            api_url: Some(api_url.to_string()),
            workspace_id: None,
            workspace_name: None,
            key_prefix: None,
        });
    };
    let personal = personal_id.and_then(|id| rows.into_iter().find(|workspace| workspace.id == id));
    Ok(StatusReport {
        authenticated: true,
        api_url: Some(api_url.to_string()),
        workspace_id: personal_id.map(str::to_string),
        workspace_name: personal.map(|info| info.name),
        key_prefix: None,
    })
}

/// `GET /v1/workspaces` with a device key. `Ok(None)` = the key no longer
/// authenticates (401/403), which callers report rather than raise.
pub fn list_workspaces(api_url: &str, secret: &str) -> Result<Option<Vec<WorkspaceInfo>>> {
    let url = format!("{api_url}/v1/workspaces");
    let auth = format!("Bearer {secret}");
    let resp = fetch::exchange(&Request {
        method: "GET",
        url: &url,
        headers: &[("Authorization", auth.as_str())],
        body: None,
    })?;
    if resp.status == 401 || resp.status == 403 {
        return Ok(None);
    }
    if !(200..300).contains(&resp.status) {
        return Err(http_err(&url, resp.status, &resp.body));
    }
    let view: WorkspaceList = serde_json::from_str(&resp.body)?;
    Ok(Some(
        view.workspaces
            .into_iter()
            .map(|row| row.workspace)
            .collect(),
    ))
}

fn request_device_code(api_url: &str) -> Result<DeviceCodeResponse> {
    let url = format!("{api_url}/auth/device/code");
    let body = serde_json::json!({ "client_id": CLIENT_ID }).to_string();
    let resp = fetch::exchange(&Request {
        method: "POST",
        url: &url,
        headers: &[("Content-Type", "application/json")],
        body: Some(&body),
    })?;
    if !(200..300).contains(&resp.status) {
        return Err(http_err(&url, resp.status, &resp.body));
    }
    Ok(serde_json::from_str(&resp.body)?)
}

fn poll_access_token(api_url: &str, code: &DeviceCodeResponse) -> Result<String> {
    let url = format!("{api_url}/auth/device/token");
    let body = serde_json::json!({
        "grant_type": GRANT_TYPE,
        "device_code": code.device_code,
        "client_id": CLIENT_ID,
    })
    .to_string();
    let deadline = Instant::now() + Duration::from_secs(code.expires_in.max(1));
    let mut interval = Duration::from_secs(code.interval.unwrap_or(DEFAULT_INTERVAL_SECS).max(1));
    loop {
        if Instant::now() >= deadline {
            return Err(Error::Unavailable(
                "device authorization timed out before approval".into(),
            ));
        }
        thread::sleep(interval);
        let resp = fetch::exchange(&Request {
            method: "POST",
            url: &url,
            headers: &[("Content-Type", "application/json")],
            body: Some(&body),
        })?;
        if (200..300).contains(&resp.status) {
            return parse_access_token(&resp.body);
        }
        let err = token_error(&resp.body);
        match err.as_deref() {
            Some("authorization_pending") => {}
            Some("slow_down") => {
                interval += Duration::from_secs(5);
            }
            Some(other) => {
                return Err(Error::Unavailable(format!(
                    "device authorization failed: {other}"
                )));
            }
            None => return Err(http_err(&url, resp.status, &resp.body)),
        }
    }
}

fn mint_device_key(api_url: &str, access_token: &str, device_name: &str) -> Result<MintResponse> {
    let url = format!("{api_url}/v1/device/mint-device-key");
    let auth = format!("Bearer {access_token}");
    let body = serde_json::json!({ "deviceName": device_name }).to_string();
    let resp = fetch::exchange(&Request {
        method: "POST",
        url: &url,
        headers: &[
            ("Authorization", auth.as_str()),
            ("Content-Type", "application/json"),
        ],
        body: Some(&body),
    })?;
    if resp.status != 201 && !(200..300).contains(&resp.status) {
        return Err(http_err(&url, resp.status, &resp.body));
    }
    Ok(serde_json::from_str(&resp.body)?)
}

fn parse_access_token(body: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct TokenOk {
        access_token: String,
    }
    let parsed: TokenOk = serde_json::from_str(body)?;
    if parsed.access_token.is_empty() {
        return Err(Error::Unavailable(
            "device token response missing access_token".into(),
        ));
    }
    Ok(parsed.access_token)
}

fn token_error(body: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct TokenErr {
        error: String,
    }
    serde_json::from_str::<TokenErr>(body).ok().map(|e| e.error)
}

fn http_err(url: &str, status: u16, body: &str) -> Error {
    let detail = body.trim();
    if detail.is_empty() {
        Error::Unavailable(format!("HTTP {status} from {url}"))
    } else {
        Error::Unavailable(format!("HTTP {status} from {url}: {detail}"))
    }
}

#[cfg(test)]
#[path = "tests/device.rs"]
mod tests;
