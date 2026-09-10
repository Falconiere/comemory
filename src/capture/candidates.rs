//! Platform candidate-batch types and `POST /v1/sessions/{id}/candidates`.

use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

use crate::capture::explicit_save::{EXTRACTOR_ID, EXTRACTOR_VERSION, ExtractedCandidate};
use crate::capture::redact::{RedactionAttestation, RedactionFinding, merge_findings, redact_text};
use crate::prelude::*;

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Platform cap: aggregate free-text **bytes** across the batch.
pub const MAX_AGGREGATE_FREE_TEXT: usize = 131_072;
/// Platform cap: candidates per request.
pub const MAX_CANDIDATES: usize = 50;

/// One proposal as the ingest route accepts it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateProposal {
    /// Product kind.
    pub kind: String,
    /// Claim title.
    pub title: String,
    /// Optional body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Optional tags.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Optional repo (defaults to the session's on the platform).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Extractor confidence in `[0, 1]`.
    pub confidence: f64,
}

/// Ready-to-post ingest batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateBatch {
    /// Extractor id (`claude-code-explicit-save`).
    pub extractor: String,
    /// Extractor version.
    pub extractor_version: u32,
    /// Client redaction attestation.
    pub redaction: RedactionAttestation,
    /// Proposals (1…50 after a non-empty check on the platform; empty OK locally).
    pub candidates: Vec<CandidateProposal>,
}

/// One per-item result from the platform, in request order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposeResult {
    /// `stored` or `duplicate`.
    pub status: String,
    /// Candidate id (existing on duplicate).
    #[serde(default)]
    pub candidate_id: Option<String>,
    /// Content digest.
    #[serde(default)]
    pub content_digest: Option<String>,
    /// Current state (`pending`, `rejected`, …).
    #[serde(default)]
    pub state: Option<String>,
}

/// Envelope data for a successful propose call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposeResponse {
    /// Per-item results in request order.
    pub results: Vec<ProposeResult>,
    /// How many were newly stored.
    #[serde(default)]
    pub stored: u32,
    /// How many collapsed as duplicates.
    #[serde(default)]
    pub duplicates: u32,
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

/// Build a redacted batch from extracted claims.
pub fn batch_from_extracted(extracted: &[ExtractedCandidate]) -> Result<CandidateBatch> {
    crate::capture::redact::ensure_rules_loaded()?;
    if extracted.len() > MAX_CANDIDATES {
        return Err(Error::Usage(format!(
            "distill produced {} candidates; platform cap is {MAX_CANDIDATES}",
            extracted.len()
        )));
    }

    let mut finding_parts: Vec<Vec<RedactionFinding>> = Vec::new();
    let mut candidates = Vec::with_capacity(extracted.len());
    let mut aggregate = 0usize;

    for claim in extracted {
        let title_out = redact_text(&claim.title);
        finding_parts.push(title_out.findings);
        // Field caps are Unicode scalars (platform wire); the aggregate cap
        // below is bytes (HTTP body budget). Keep them separate on purpose.
        let mut title = title_out.text;
        if title.chars().count() > 200 {
            title = title.chars().take(200).collect();
        }
        aggregate = aggregate.saturating_add(title.len());

        let body = match &claim.body {
            Some(raw) => {
                let body_out = redact_text(raw);
                finding_parts.push(body_out.findings);
                let mut text = body_out.text;
                if text.chars().count() > 4_000 {
                    text = text.chars().take(4_000).collect();
                }
                aggregate = aggregate.saturating_add(text.len());
                if text.is_empty() { None } else { Some(text) }
            }
            None => None,
        };

        let tags = if claim.tags.is_empty() {
            None
        } else {
            let mut cleaned = Vec::new();
            for tag in claim.tags.iter().take(16) {
                let tag_out = redact_text(tag);
                finding_parts.push(tag_out.findings);
                let mut t = tag_out.text;
                if t.chars().count() > 64 {
                    t = t.chars().take(64).collect();
                }
                aggregate = aggregate.saturating_add(t.len());
                if !t.is_empty() {
                    cleaned.push(t);
                }
            }
            if cleaned.is_empty() {
                None
            } else {
                Some(cleaned)
            }
        };

        candidates.push(CandidateProposal {
            kind: claim.kind.clone(),
            title,
            body,
            tags,
            repo: None,
            confidence: claim.confidence.clamp(0.0, 1.0),
        });
    }

    if aggregate > MAX_AGGREGATE_FREE_TEXT {
        return Err(Error::Usage(format!(
            "distill free text is {aggregate} bytes; platform cap is {MAX_AGGREGATE_FREE_TEXT}"
        )));
    }

    Ok(CandidateBatch {
        extractor: EXTRACTOR_ID.to_string(),
        extractor_version: EXTRACTOR_VERSION,
        redaction: merge_findings(&finding_parts),
        candidates,
    })
}

/// POST the batch to the platform.
pub fn post_candidates(
    api_url: &str,
    org_key: &str,
    session_id: &str,
    batch: &CandidateBatch,
) -> Result<ProposeResponse> {
    crate::capture::redact::ensure_rules_loaded()?;
    validate_session_id(session_id)?;
    let base = api_url.trim_end_matches('/');
    let url = format!("{base}/v1/sessions/{session_id}/candidates");
    let client = Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| Error::Other(format!("http client: {e}")))?;
    let resp = client
        .post(&url)
        .headers(auth_headers(org_key)?)
        .json(batch)
        .send()
        .map_err(|e| Error::Other(format!("http: {e}")))?;
    parse_envelope(resp, "post candidates")
}

fn auth_headers(org_key: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let value = HeaderValue::from_str(&format!("Bearer {org_key}"))
        .map_err(|e| Error::Other(format!("authorization header: {e}")))?;
    headers.insert(AUTHORIZATION, value);
    Ok(headers)
}

/// Session ids are path segments; reject empty, oversized, or traversal-ish values.
fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.is_empty() || session_id.len() > 128 {
        return Err(Error::Usage("session id must be 1..=128 characters".into()));
    }
    let ok = session_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err(Error::Usage(
            "session id must be alphanumeric, hyphen, or underscore".into(),
        ));
    }
    Ok(())
}

fn parse_envelope<T: for<'de> Deserialize<'de>>(
    resp: reqwest::blocking::Response,
    ctx: &str,
) -> Result<T> {
    let status = resp.status();
    let text = resp
        .text()
        .map_err(|e| Error::Other(format!("http: {e}")))?;
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
#[path = "tests/candidates.rs"]
mod tests;
