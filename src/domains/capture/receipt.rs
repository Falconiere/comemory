//! Build a platform capture receipt from a loaded transcript.

use sha2::{Digest, Sha256};
use std::fmt::Write as _;

use crate::prelude::*;

use super::claude_code::ClaudeSession;
use super::client::SessionReceipt;
use super::redact::{ensure_rules_loaded, merge_findings, redact_text};

/// Apply client redaction and assemble the wire receipt (body never leaves).
pub fn build_receipt(session: &ClaudeSession, source: &str) -> Result<SessionReceipt> {
    ensure_rules_loaded()?;
    let raw_text = std::str::from_utf8(&session.raw)
        .map_err(|e| Error::Usage(format!("transcript not UTF-8: {e}")))?;
    let body = redact_text(raw_text);
    let title_out = redact_text(&session.title);
    let redaction = merge_findings(&[body.findings, title_out.findings.clone()]);
    let digest = sha256_hex(body.text.as_bytes());
    let transcript_bytes = u64::try_from(body.text.len())
        .map_err(|_| Error::Other("transcript byte length exceeds u64".into()))?;
    if transcript_bytes == 0 || transcript_bytes > 268_435_456 {
        return Err(Error::Usage(format!(
            "transcriptBytes {transcript_bytes} out of range 1..=268435456"
        )));
    }
    let title = if title_out.text.chars().count() > 200 {
        title_out.text.chars().take(200).collect()
    } else {
        title_out.text
    };

    Ok(SessionReceipt {
        source: source.to_string(),
        external_id: session.external_id.clone(),
        title: Some(title).filter(|s| !s.is_empty()),
        repo: session.repo.clone(),
        branch: session.branch.clone(),
        tool_version: session.tool_version.clone(),
        started_at: session.started_at.clone(),
        ended_at: session.ended_at.clone(),
        turn_count: session.turn_count,
        transcript_bytes,
        transcript_digest: digest,
        redaction,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in out {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}
