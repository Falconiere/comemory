//! Blocking HTTP client for capture consent + session receipts.

use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};

use crate::prelude::*;

use super::redact::RedactionAttestation;

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const WORKSPACE_HEADER: &str = "x-comemory-workspace";

/// `POST /v1/sessions` request body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionReceipt {
    /// Capture source enum member (`claude-code`, …).
    pub source: String,
    /// Tool-native session id.
    pub external_id: String,
    /// First user turn, ≤ 200 chars, redacted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Repo label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Git branch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Coding-tool version string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,
    /// ISO 8601 start.
    pub started_at: String,
    /// ISO 8601 end.
    pub ended_at: String,
    /// User + assistant turn count.
    pub turn_count: u32,
    /// Byte length of the **redacted** transcript.
    pub transcript_bytes: u64,
    /// SHA-256 hex of the redacted transcript.
    pub transcript_digest: String,
    /// Redaction attestation.
    pub redaction: RedactionAttestation,
}

/// One row from `GET /v1/capture/sources`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSourceRow {
    /// Source id.
    pub source: String,
    /// Whether capture is consented.
    pub enabled: bool,
    /// Last consent change, or null when never set.
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SourcesResponse {
    sources: Vec<CaptureSourceRow>,
}

/// `POST /v1/sessions` success payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PostSessionResponse {
    /// Stored/updated session row (opaque JSON object from the platform).
    pub session: serde_json::Value,
    /// True when this call inserted the row.
    pub created: bool,
}

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

fn http_client() -> Result<Client> {
    Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| Error::Other(format!("http client: {e}")))
}

fn normalize_api_url(api_url: &str) -> String {
    api_url.trim_end_matches('/').to_string()
}

fn auth_headers(secret: &str, workspace_id: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    // Do not include `from_str` Display in the error — it can echo the secret.
    let bearer = HeaderValue::from_str(&format!("Bearer {secret}"))
        .map_err(|_| Error::Other("authorization header: invalid secret format".into()))?;
    headers.insert(AUTHORIZATION, bearer);
    let ws = HeaderValue::from_str(workspace_id)
        .map_err(|_| Error::Other("workspace header: invalid workspace id".into()))?;
    headers.insert(HeaderName::from_static(WORKSPACE_HEADER), ws);
    Ok(headers)
}

fn map_reqwest(e: reqwest::Error) -> Error {
    Error::Other(format!("http: {e}"))
}

fn parse_envelope<T: for<'de> Deserialize<'de>>(
    resp: reqwest::blocking::Response,
    ctx: &str,
) -> Result<T> {
    let status = resp.status();
    let text = resp.text().map_err(map_reqwest)?;
    if !status.is_success() {
        if let Ok(env) = serde_json::from_str::<ApiEnvelope<T>>(&text)
            && let Some(err) = env.error
        {
            return Err(Error::Other(format!(
                "{ctx}: HTTP {status}: {} — {}",
                err.code, err.message
            )));
        }
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

/// `GET /v1/capture/sources`.
pub fn list_sources(
    api_url: &str,
    secret: &str,
    workspace_id: &str,
) -> Result<Vec<CaptureSourceRow>> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/capture/sources");
    let client = http_client()?;
    let resp = client
        .get(&url)
        .headers(auth_headers(secret, workspace_id)?)
        .send()
        .map_err(map_reqwest)?;
    let body: SourcesResponse = parse_envelope(resp, "capture sources")?;
    Ok(body.sources)
}

/// `POST /v1/sessions`.
pub fn post_session(
    api_url: &str,
    secret: &str,
    workspace_id: &str,
    receipt: &SessionReceipt,
) -> Result<PostSessionResponse> {
    let base = normalize_api_url(api_url);
    let url = format!("{base}/v1/sessions");
    let client = http_client()?;
    let resp = client
        .post(&url)
        .headers(auth_headers(secret, workspace_id)?)
        .json(receipt)
        .send()
        .map_err(map_reqwest)?;
    parse_envelope(resp, "post session")
}
