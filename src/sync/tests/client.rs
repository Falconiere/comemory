#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! Real HTTP coverage of `comemory::sync::client` against the loopback
//! platform fixture (`tests/common/sync_platform_server.rs`).

use comemory::api::sync::{ImportEntry, ImportRequest, ImportStatus, SyncOp};
use comemory::sync::client;

use crate::test_common::sync_platform_server::{SyncPlatformServer, SyncPlatformState};

/// One import entry, so the header assertions below exercise a real POST body.
fn sample_entry() -> ImportRequest {
    ImportRequest {
        cursor: 0,
        entries: vec![ImportEntry {
            op: SyncOp::Upsert,
            id: "abcd1234".into(),
            content_hash: "aa".repeat(32),
            at: "2026-09-06T12:00:00Z".into(),
            record: None,
        }],
    }
}

#[test]
fn device_code_and_token_roundtrip() {
    let server = SyncPlatformServer::start_default();
    let code = client::device_code(&server.base).expect("device code");
    assert_eq!(code.device_code, "dc-1");
    assert_eq!(code.interval, 1);

    let token = client::poll_token(&server.base, &code.device_code).expect("token");
    assert_eq!(token.access_token.as_deref(), Some("dev-access-token"));
}

#[test]
fn sends_no_workspace_header_on_any_sync_route() {
    // The org key already names the workspace. If the header came back, a
    // caller could ask for a workspace the key cannot reach — so this asserts
    // on what the server actually received, not on the client's intent.
    let platform = SyncPlatformState {
        head_seq: 1,
        ..Default::default()
    };
    let server = SyncPlatformServer::start(platform);
    let secret = server.snapshot().secret;

    client::pull_changes(&server.base, &secret, 0, 50).expect("changes");
    client::push_import(&server.base, &secret, &sample_entry()).expect("import");
    client::fetch_manifest(&server.base, &secret).expect("manifest");

    let seen = server.requests();
    assert_eq!(seen.len(), 3, "three sync calls, got: {seen:?}");
    for request in &seen {
        assert!(
            request.workspace_header.is_none(),
            "{} {} sent the header at all — absence is the contract, and an \
             empty value would still be a regression: {:?}",
            request.method,
            request.path,
            request.workspace_header
        );
        assert_eq!(
            request.authorization,
            format!("Bearer {secret}"),
            "every sync call authenticates with the org key"
        );
    }
}

#[test]
fn never_calls_the_removed_allowlist_route() {
    // `/v1/sync/status` was the second source of truth beside org membership.
    // A regression that reintroduced it would still pass a body-only
    // assertion, so this checks the request log.
    let server = SyncPlatformServer::start_default();
    let secret = server.snapshot().secret;

    client::pull_changes(&server.base, &secret, 0, 50).expect("changes");
    client::push_import(&server.base, &secret, &sample_entry()).expect("import");

    assert!(
        !server.saw_path("/v1/sync/status"),
        "the allowlist route must never be reached, saw: {:?}",
        server.paths()
    );
    assert!(
        !server.saw_path("/v1/workspaces"),
        "an org key has no workspace list to fetch, saw: {:?}",
        server.paths()
    );
}

#[test]
fn pull_changes_push_import_and_manifest() {
    let platform = SyncPlatformState {
        head_seq: 3,
        changes: serde_json::json!([{
            "seq": 3,
            "op": "tombstone",
            "id": "deadbeef",
            "content_hash": "00".repeat(32),
            "at": "2026-09-06T12:00:00Z",
            "author": "peer",
            "record": null
        }]),
        import_results: serde_json::json!([{
            "id": "abcd1234",
            "content_hash": "aa".repeat(32),
            "status": "accepted"
        }]),
        ..Default::default()
    };
    let server = SyncPlatformServer::start(platform);
    let secret = server.snapshot().secret;

    let changes = client::pull_changes(&server.base, &secret, 0, 50).expect("changes");
    assert_eq!(changes.entries.len(), 1);
    assert_eq!(changes.head_seq, 3);
    assert_eq!(changes.entries[0].op, SyncOp::Tombstone);

    let import = client::push_import(&server.base, &secret, &sample_entry()).expect("import");
    assert_eq!(import.results.len(), 1);
    assert_eq!(import.results[0].status, ImportStatus::Accepted);

    let manifest = client::fetch_manifest(&server.base, &secret).expect("manifest");
    assert_eq!(manifest.buckets.len(), 256);
    assert_eq!(manifest.head_seq, 3);
}

#[test]
fn envelope_error_surfaces_with_its_code() {
    let platform = SyncPlatformState {
        import_results: serde_json::json!([]),
        ..Default::default()
    };
    let server = SyncPlatformServer::start(platform);
    server.update(|st| st.sync_unavailable = true);
    let secret = server.snapshot().secret;

    // The fixture's `unavailable()` answers exactly `500 Internal Server
    // Error`, so the status is pinned on both sides of this assertion.
    let err = client::pull_changes(&server.base, &secret, 0, 50).expect_err("outage surfaces");
    let msg = err.to_string();
    assert!(
        msg.contains("500"),
        "the HTTP status must reach the caller: {msg}"
    );
    assert!(
        msg.contains("pull changes"),
        "and so must the operation that failed: {msg}"
    );
}

#[test]
fn trailing_slash_base_url_normalizes() {
    let server = SyncPlatformServer::start_default();
    let secret = server.snapshot().secret;
    let base = format!("{}/", server.base);
    let manifest = client::fetch_manifest(&base, &secret).expect("slash");
    assert_eq!(manifest.buckets.len(), 256);
}

/// Send `headers` verbatim over a raw socket and return what the fixture
/// recorded for that request.
///
/// Raw bytes because the point is to control the exact header lines; a client
/// library would normalize them out of existence.
fn record_raw(server: &SyncPlatformServer, headers: &[&str]) -> Option<String> {
    use std::io::{Read as _, Write as _};

    let before = server.requests().len();
    let addr = server.base.trim_start_matches("http://");
    let mut stream = std::net::TcpStream::connect(addr).expect("connect to fixture");
    let mut req = String::from("GET /v1/sync/manifest HTTP/1.1\r\nHost: fixture\r\n");
    for header in headers {
        req.push_str(header);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    stream.write_all(req.as_bytes()).expect("write request");
    let mut sink = Vec::new();
    let _ = stream.read_to_end(&mut sink);

    let seen = server.requests();
    assert_eq!(
        seen.len(),
        before + 1,
        "the fixture must log exactly this request"
    );
    seen[before].workspace_header.clone()
}

#[test]
fn header_capture_matches_the_workspace_header_and_only_that_header() {
    // This fixture's capture is what AC-11 rests on: if it silently failed to
    // record a header that WAS sent, "no request carries a workspace header"
    // would pass while the client regressed. So the capture itself is pinned
    // here, including the near-miss name that shares its prefix.
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;
    let auth = format!("Authorization: Bearer {secret}");

    // A different header whose name merely starts with the same text must not
    // be mistaken for it — the prefix ends in `:`, so matching stops at the `-`.
    assert_eq!(
        record_raw(&server, &[&auth, "X-Comemory-Workspace-Extra: poison"]),
        None,
        "a longer header name sharing the prefix must not be captured"
    );

    // The real header is captured, with or without the optional space after
    // the colon (RFC 9112 §5: `field-line = field-name \":\" OWS field-value OWS`).
    assert_eq!(
        record_raw(&server, &[&auth, "X-Comemory-Workspace: ws-spaced"]),
        Some("ws-spaced".to_string())
    );
    assert_eq!(
        record_raw(&server, &[&auth, "X-Comemory-Workspace:ws-tight"]),
        Some("ws-tight".to_string()),
        "the space after the colon is optional in HTTP and must not be required here"
    );

    // Case-insensitive on the name, value preserved verbatim.
    assert_eq!(
        record_raw(&server, &[&auth, "x-COMEMORY-workspace: WS-MixedCase"]),
        Some("WS-MixedCase".to_string())
    );

    // And absence stays absence.
    assert_eq!(record_raw(&server, &[&auth]), None);
}
