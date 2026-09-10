//! RFC 8628 device authorization + organization-key mint against the platform.
//!
//! Flow: `POST /auth/device/code` → print user code / URI → poll
//! `POST /auth/device/token` → `POST /v1/device/mint-org-key` with
//! `Authorization: Bearer <access_token>`. The device-code grant is only the
//! approval channel; what it mints is scoped to the organization the user
//! approved, so the workspace travels in the key and no call names one.
//!
//! Which organization is decided in the console at approval time, so the CLI
//! never prompts and the mint returns exactly one.

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

/// One-shot mint payload from `POST /v1/device/mint-org-key`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MintResponse {
    secret: String,
    key_prefix: String,
    organization_id: String,
    #[serde(default)]
    organization_slug: String,
    #[serde(default)]
    organization_name: String,
    workspace_id: String,
    api_url: String,
    #[serde(default)]
    email: Option<String>,
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
    /// Organization the key is scoped to, echoed from `auth.json`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    /// Organization display name, echoed from `auth.json`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_name: Option<String>,
    /// The organization workspace the key can reach.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Key prefix for display (never the full secret).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_prefix: Option<String>,
}

/// Run the full device login against `api_url`, printing the user code to
/// `progress` (typically stderr). Returns the minted organization credentials
/// for the caller to persist.
///
/// # Errors
/// [`Error::Unavailable`] when the grant fails, times out, or the mint comes
/// back without an organization scope.
pub fn login(api_url: &str, progress: &mut impl Write) -> Result<LoginOutcome> {
    let code = request_device_code(api_url)?;
    let uri = code
        .verification_uri_complete
        .as_deref()
        .unwrap_or(code.verification_uri.as_str());
    writeln!(progress, "Visit {uri}\nand enter code: {}", code.user_code)?;
    let _ = progress.flush();
    let access_token = poll_access_token(api_url, &code)?;
    let minted = mint_org_key(api_url, &access_token)?;
    // A key with no organization or no workspace can do nothing, so refuse it
    // here rather than write a credential every later call would reject.
    if minted.organization_id.trim().is_empty() || minted.workspace_id.trim().is_empty() {
        return Err(Error::Unavailable(
            "mint response missing organization scope (organizationId / workspaceId)".into(),
        ));
    }
    // The platform echoes its own canonical base; an empty one means the
    // deployment did not set `BETTER_AUTH_URL`, so keep the URL we dialed.
    let resolved_api_url = if minted.api_url.trim().is_empty() {
        api_url.to_string()
    } else {
        minted.api_url
    };
    Ok(LoginOutcome {
        credentials: AuthFile {
            version: crate::sync::auth_file::AUTH_SCHEMA_VERSION,
            secret: minted.secret,
            key_prefix: minted.key_prefix,
            api_url: resolved_api_url,
            organization_id: minted.organization_id,
            organization_slug: minted.organization_slug,
            organization_name: minted.organization_name,
            workspace_id: minted.workspace_id,
            email: minted.email,
        },
    })
}

/// Probe whether `secret` still authenticates against `api_url`, reporting the
/// organization recorded in `auth.json`.
///
/// The probe is `GET /v1/sync/manifest`: an org key's own surface, and the
/// cheapest call that proves the key is accepted for what it is actually used
/// for. A `401`/`403` is reported as `authenticated: false` rather than raised
/// — a revoked key is an answer, not a crash.
///
/// # Errors
/// [`Error::Unavailable`] when the platform is unreachable or answers a status
/// that is neither success nor an auth rejection.
pub fn org_status(
    api_url: &str,
    secret: &str,
    recorded: Option<&AuthFile>,
) -> Result<StatusReport> {
    let url = format!("{api_url}/v1/sync/manifest");
    let auth = format!("Bearer {secret}");
    let resp = fetch::exchange(&Request {
        method: "GET",
        url: &url,
        headers: &[("Authorization", auth.as_str())],
        body: None,
    })?;
    if resp.status == 401 || resp.status == 403 {
        return Ok(StatusReport {
            authenticated: false,
            api_url: Some(api_url.to_string()),
            organization_id: None,
            organization_name: None,
            workspace_id: None,
            key_prefix: None,
        });
    }
    if !(200..300).contains(&resp.status) {
        return Err(http_err(&url, resp.status, &resp.body));
    }
    Ok(StatusReport {
        authenticated: true,
        api_url: Some(api_url.to_string()),
        organization_id: recorded.map(|a| a.organization_id.clone()),
        organization_name: recorded.map(|a| a.organization_name.clone()),
        workspace_id: recorded.map(|a| a.workspace_id.clone()),
        key_prefix: recorded.map(|a| a.key_prefix.clone()),
    })
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

fn mint_org_key(api_url: &str, access_token: &str) -> Result<MintResponse> {
    let url = format!("{api_url}/v1/device/mint-org-key");
    let auth = format!("Bearer {access_token}");
    let body = "{}".to_string();
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
