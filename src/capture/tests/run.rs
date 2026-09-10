#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Capture orchestration + platform client against the loopback fixture.

#[path = "../../../tests/common/capture_platform_server.rs"]
mod capture_platform_server;

use std::path::PathBuf;

use capture_platform_server::{CapturePlatformServer, CapturePlatformState};
use comemory::capture::{CaptureRequest, run_capture, run_sources};
use comemory::sync::AuthFile;

fn fixture() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/common/fixtures/claude-code-session.jsonl"
    ))
}

fn auth(api_url: &str) -> AuthFile {
    AuthFile {
        version: 2,
        secret: "cmk_testsecret".into(),
        key_prefix: "cmk_test".into(),
        api_url: api_url.into(),
        organization_id: "org".into(),
        organization_slug: "org".into(),
        organization_name: "Org".into(),
        workspace_id: "ws-org".into(),
        email: None,
    }
}

#[test]
fn sources_lists_consent_rows() {
    let server = CapturePlatformServer::spawn(CapturePlatformState::default());
    let rows = run_sources(&auth(&server.base_url)).expect("sources");
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0].source, "claude-code");
    assert!(rows[0].enabled);
    let reqs = server.requests();
    assert_eq!(reqs[0].path, "/v1/capture/sources");
    assert_eq!(reqs[0].workspace_header.as_deref(), Some("ws-org"));
    assert!(reqs[0].authorization.starts_with("Bearer "));
}

#[test]
fn dry_run_builds_receipt_without_post() {
    let server = CapturePlatformServer::spawn(CapturePlatformState::default());
    let report = run_capture(
        &auth(&server.base_url),
        CaptureRequest {
            source: "claude-code".into(),
            path: Some(fixture()),
            session_id: None,
            dry_run: true,
            allow_secret: true,
        },
    )
    .expect("dry-run");
    assert!(report.dry_run);
    assert!(!report.posted);
    assert_eq!(report.receipt.redaction.version, 1);
    assert!(server.requests().is_empty());
}

#[test]
fn post_sends_receipt_and_workspace_header() {
    let server = CapturePlatformServer::spawn(CapturePlatformState::default());
    let report = run_capture(
        &auth(&server.base_url),
        CaptureRequest {
            source: "claude-code".into(),
            path: Some(fixture()),
            session_id: None,
            dry_run: false,
            allow_secret: true,
        },
    )
    .expect("post");
    assert!(report.posted);
    assert!(report.response.as_ref().unwrap().created);
    let reqs = server.requests();
    let post = reqs
        .iter()
        .find(|r| r.method == "POST" && r.path == "/v1/sessions")
        .expect("POST /v1/sessions");
    assert_eq!(post.workspace_header.as_deref(), Some("ws-org"));
    let body = server.last_receipt().unwrap();
    let receipt: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        receipt
            .get("externalId")
            .and_then(serde_json::Value::as_str),
        Some("8e9f54e3-a984-46ab-8403-135ee920cbca")
    );
    assert_eq!(
        receipt
            .get("transcriptDigest")
            .and_then(serde_json::Value::as_str)
            .map(str::len),
        Some(64)
    );
    assert_eq!(
        receipt
            .pointer("/redaction/version")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
}

#[test]
fn unknown_source_is_usage_error() {
    let server = CapturePlatformServer::spawn(CapturePlatformState::default());
    let err = run_capture(
        &auth(&server.base_url),
        CaptureRequest {
            source: "cursor".into(),
            path: Some(fixture()),
            session_id: None,
            dry_run: true,
            allow_secret: true,
        },
    )
    .expect_err("cursor");
    let msg = err.to_string();
    assert!(msg.contains("not implemented"), "{msg}");
}
