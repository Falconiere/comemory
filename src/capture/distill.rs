//! Orchestrate extract → redact → POST (or dry-run) for `comemory distill`.

use std::path::PathBuf;

use serde::Serialize;

use crate::capture::candidates::{
    CandidateBatch, ProposeResponse, batch_from_extracted, post_candidates,
};
use crate::capture::claude_code::{bash_commands_from_jsonl, read_transcript_file};
use crate::capture::explicit_save::extract_explicit_saves;
use crate::prelude::*;
use crate::sync::auth_file::AuthFile;

/// Inputs for one distill run.
#[derive(Debug, Clone)]
pub struct DistillRequest {
    /// Platform session id from a prior capture receipt.
    pub session_id: String,
    /// Path to a Claude Code JSONL transcript.
    pub transcript: PathBuf,
    /// When true, build the batch but skip the HTTP POST.
    pub dry_run: bool,
    /// Optional API base override (`--api-url`); else `auth.api_url`.
    pub api_url: Option<String>,
}

/// Outcome of a distill run (posted or dry-run).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DistillReport {
    /// Session the batch was (or would be) posted against.
    pub session_id: String,
    /// How many claims the extractor recovered.
    pub extracted: usize,
    /// The batch that was posted or would be posted.
    pub batch: CandidateBatch,
    /// Platform response when not dry-run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<ProposeResponse>,
    /// True when no POST was made.
    pub dry_run: bool,
}

/// Extract and redact a transcript into a candidate batch (no network).
pub fn build_batch(transcript: &std::path::Path) -> Result<(usize, CandidateBatch)> {
    let text = read_transcript_file(transcript)?;
    let commands = bash_commands_from_jsonl(&text);
    let extracted = extract_explicit_saves(&commands);
    let batch = batch_from_extracted(&extracted)?;
    Ok((extracted.len(), batch))
}

/// Extract, redact, and optionally POST candidates for one session transcript.
pub fn run(auth: Option<&AuthFile>, req: &DistillRequest) -> Result<DistillReport> {
    let (extracted, batch) = build_batch(&req.transcript)?;

    if req.dry_run {
        return Ok(DistillReport {
            session_id: req.session_id.clone(),
            extracted,
            batch,
            response: None,
            dry_run: true,
        });
    }

    let auth =
        auth.ok_or_else(|| Error::Usage("not logged in — run `comemory auth login`".into()))?;

    if batch.candidates.is_empty() {
        return Ok(DistillReport {
            session_id: req.session_id.clone(),
            extracted: 0,
            batch,
            response: Some(ProposeResponse {
                results: Vec::new(),
                stored: 0,
                duplicates: 0,
            }),
            dry_run: false,
        });
    }

    let api_url = req.api_url.clone().unwrap_or_else(|| auth.api_url.clone());
    let secret = auth.effective_secret();
    let response = post_candidates(&api_url, &secret, &req.session_id, &batch)?;
    Ok(DistillReport {
        session_id: req.session_id.clone(),
        extracted,
        batch,
        response: Some(response),
        dry_run: false,
    })
}
