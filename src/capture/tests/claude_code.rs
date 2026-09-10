#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Claude Code adapter over the committed real-session fixture.

use std::path::PathBuf;

use comemory::capture::claude_code;
use sha2::{Digest, Sha256};

fn fixture() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/common/fixtures/claude-code-session.jsonl"
    ))
}

#[test]
fn loads_fixture_session_metadata() {
    let s = claude_code::load_path(&fixture()).expect("load");
    assert_eq!(s.external_id, "8e9f54e3-a984-46ab-8403-135ee920cbca");
    assert_eq!(s.tool_version.as_deref(), Some("2.1.267"));
    assert_eq!(s.branch.as_deref(), Some("main"));
    assert_eq!(s.repo.as_deref(), Some("comemory.io"));
    assert_eq!(s.turn_count, 24);
    assert_eq!(s.started_at, "2026-09-10T07:40:49.248Z");
    assert!(s.title.contains("search for missing"));
}

#[test]
fn receipt_digest_matches_redacted_bytes() {
    let s = claude_code::load_path(&fixture()).expect("load");
    let receipt = comemory::capture::build_receipt(&s, "claude-code").expect("receipt");
    assert_eq!(receipt.redaction.version, 1);
    assert_eq!(receipt.turn_count, 24);
    let raw = std::str::from_utf8(&s.raw).unwrap();
    let redacted = comemory::capture::redact_text(raw).text;
    assert_eq!(receipt.transcript_bytes, redacted.len() as u64);
    let mut hasher = Sha256::new();
    hasher.update(redacted.as_bytes());
    let digest = hasher.finalize();
    let mut expect = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(expect, "{b:02x}");
    }
    assert_eq!(receipt.transcript_digest, expect);
}
