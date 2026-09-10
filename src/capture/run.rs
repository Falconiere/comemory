//! Orchestrate capture: load transcript → receipt → optional POST.

use std::path::PathBuf;

use serde::Serialize;

use crate::prelude::*;
use crate::sync::AuthFile;

use super::claude_code;
use super::client::{self, CaptureSourceRow, PostSessionResponse, SessionReceipt};
use super::receipt::build_receipt;

/// Supported capture sources. Only `claude-code` is implemented in this slice.
pub const SOURCE_CLAUDE_CODE: &str = "claude-code";

/// Inputs for one capture run.
#[derive(Debug, Clone)]
pub struct CaptureRequest {
    /// Wire `source` field.
    pub source: String,
    /// Explicit transcript path.
    pub path: Option<PathBuf>,
    /// Tool session id (looked up under `~/.claude/projects`).
    pub session_id: Option<String>,
    /// Build the receipt but do not POST.
    pub dry_run: bool,
    /// When secrets are found, still POST (attestation still lists them).
    pub allow_secret: bool,
}

/// Result surfaced to the CLI.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureReport {
    /// Whether the receipt was POSTed.
    pub posted: bool,
    /// Dry-run flag echoed.
    pub dry_run: bool,
    /// The receipt that was (or would be) sent.
    pub receipt: SessionReceipt,
    /// Platform response when posted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<PostSessionResponse>,
}

/// List capture consent rows from the platform.
pub fn run_sources(auth: &AuthFile) -> Result<Vec<CaptureSourceRow>> {
    client::list_sources(&auth.api_url, &auth.secret, &auth.workspace_id)
}

/// Load, redact, and optionally POST one session receipt.
pub fn run_capture(auth: &AuthFile, req: CaptureRequest) -> Result<CaptureReport> {
    if req.source != SOURCE_CLAUDE_CODE {
        return Err(Error::Usage(format!(
            "capture source `{}` is not implemented yet (only `{SOURCE_CLAUDE_CODE}`)",
            req.source
        )));
    }
    let session = match (&req.path, &req.session_id) {
        (Some(path), _) => claude_code::load_path(path),
        (None, Some(id)) => claude_code::load_session_id(id),
        (None, None) => Err(Error::Usage(
            "capture requires --path or --session-id (or --from-hook)".into(),
        )),
    }?;
    let receipt = build_receipt(&session, &req.source)?;
    if !req.allow_secret && !receipt.redaction.findings.is_empty() {
        return Err(Error::Usage(format!(
            "redaction found {} secret(s) — re-run with --allow-secret to post the redacted receipt",
            receipt.redaction.findings.len()
        )));
    }
    if req.dry_run {
        return Ok(CaptureReport {
            posted: false,
            dry_run: true,
            receipt,
            response: None,
        });
    }
    let response = client::post_session(&auth.api_url, &auth.secret, &auth.workspace_id, &receipt)?;
    Ok(CaptureReport {
        posted: true,
        dry_run: false,
        receipt,
        response: Some(response),
    })
}

#[cfg(test)]
#[path = "tests/run.rs"]
mod tests;
