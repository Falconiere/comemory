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
use comemory::sync::match_key::{MatchOutcome, classify_repo};

use crate::test_common::sync_platform_server::{SyncPlatformServer, SyncPlatformState};

#[test]
fn device_code_token_and_mint_roundtrip() {
    let server = SyncPlatformServer::start_default();
    let code = client::device_code(&server.base).expect("device code");
    assert_eq!(code.device_code, "dc-1");
    assert_eq!(code.interval, 1);

    let token = client::poll_token(&server.base, &code.device_code).expect("token");
    assert_eq!(token.access_token.as_deref(), Some("dev-access-token"));

    let minted = client::mint_device_key(&server.base, "dev-access-token", "laptop").expect("mint");
    assert!(minted.secret.starts_with("cmk_"));
    assert_eq!(minted.personal_workspace_id, "ws-personal");
    assert_eq!(minted.email.as_deref(), Some("dev@example.com"));
}

#[test]
fn list_workspaces_and_allowlist_paths() {
    let server = SyncPlatformServer::start_default();
    let secret = server.snapshot().secret;

    let rows = client::list_workspaces(&server.base, &secret).expect("workspaces");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "ws-personal");
    assert_eq!(rows[0].name, "Personal");

    let (repos, etag) =
        client::fetch_allowlist(&server.base, &secret, "ws-org", None).expect("allowlist");
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].full_name, "codasignal/foo");
    assert_eq!(etag.as_deref(), Some("etag-1"));
    assert_eq!(
        classify_repo("codasignal/foo", &repos),
        MatchOutcome::Allowed
    );

    // Matching etag → empty repos, same etag (cache keep).
    let (again, etag2) =
        client::fetch_allowlist(&server.base, &secret, "ws-org", Some("etag-1")).expect("etag");
    assert!(again.is_empty());
    assert_eq!(etag2.as_deref(), Some("etag-1"));
}

#[test]
fn fetch_allowlist_404_is_empty() {
    let mut state = SyncPlatformState::default();
    state.status_404 = true;
    let server = SyncPlatformServer::start(state);
    let secret = server.snapshot().secret;
    let (repos, etag) =
        client::fetch_allowlist(&server.base, &secret, "ws", None).expect("404 status");
    assert!(repos.is_empty());
    assert!(etag.is_none());
}

#[test]
fn fetch_allowlist_envelope_error_surfaces() {
    let mut state = SyncPlatformState::default();
    state.status_error = Some(("forbidden".into(), "no sync".into()));
    let server = SyncPlatformServer::start(state);
    let secret = server.snapshot().secret;
    let err = client::fetch_allowlist(&server.base, &secret, "ws", None).expect_err("gate");
    let msg = err.to_string();
    assert!(msg.contains("forbidden"), "{msg}");
}

#[test]
fn pull_changes_push_import_and_manifest() {
    let mut state = SyncPlatformState::default();
    state.head_seq = 3;
    state.changes = serde_json::json!([{
        "seq": 3,
        "op": "tombstone",
        "id": "deadbeef",
        "content_hash": "00".repeat(32),
        "at": "2026-09-06T12:00:00Z",
        "author": "peer",
        "record": null
    }]);
    state.import_results = serde_json::json!([{
        "id": "abcd1234",
        "content_hash": "aa".repeat(32),
        "status": "repo_not_allowed"
    }]);
    let server = SyncPlatformServer::start(state);
    let secret = server.snapshot().secret;

    let changes = client::pull_changes(&server.base, &secret, "ws-org", 0, 50).expect("changes");
    assert_eq!(changes.entries.len(), 1);
    assert_eq!(changes.head_seq, 3);
    assert_eq!(changes.entries[0].op, SyncOp::Tombstone);

    let import = client::push_import(
        &server.base,
        &secret,
        "ws-org",
        &ImportRequest {
            cursor: 0,
            entries: vec![ImportEntry {
                op: SyncOp::Upsert,
                id: "abcd1234".into(),
                content_hash: "aa".repeat(32),
                at: "2026-09-06T12:00:00Z".into(),
                record: None,
            }],
        },
    )
    .expect("import");
    assert_eq!(import.results.len(), 1);
    assert_eq!(import.results[0].status, ImportStatus::RepoNotAllowed);

    let manifest = client::fetch_manifest(&server.base, &secret, "ws-org").expect("manifest");
    assert_eq!(manifest.buckets.len(), 256);
    assert_eq!(manifest.head_seq, 3);
}

#[test]
fn trailing_slash_base_url_normalizes() {
    let server = SyncPlatformServer::start_default();
    let secret = server.snapshot().secret;
    let base = format!("{}/", server.base);
    let (repos, _) = client::fetch_allowlist(&base, &secret, "ws", None).expect("slash");
    assert_eq!(repos.len(), 1);
}
