#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! Wire-shape fixtures matching the platform OpenAPI / sync envelopes
//! (`2026-09-02-memory-sync-design.md` AC-14…17 client parse path).

use comemory::api::sync::{ImportResponse, ImportStatus};
use comemory::sync::match_key::{AllowlistRepo, MatchOutcome, classify_repo};

/// Platform `GET /v1/sync/status` success body (allowlist on status, camelCase).
const STATUS_ENVELOPE: &str = r#"{
  "ok": true,
  "data": {
    "head_seq": 0,
    "devices": [],
    "allowlist": [
      {"fullName": "codasignal/foo", "name": "foo"},
      {"fullName": "org/cli", "name": "cli"},
      {"fullName": "other/cli", "name": "cli"}
    ],
    "allowlist_etag": "codasignal/foo|org/cli|other/cli",
    "personal_sync": false
  },
  "meta": {"command": "platform"}
}"#;

/// Worker gate rejection for a forged import (AC-16).
const GATED_IMPORT: &str = r#"{
  "ok": true,
  "data": {
    "results": [
      {
        "id": "abcd1234",
        "content_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "status": "repo_not_allowed"
      }
    ],
    "head_seq": 0
  },
  "meta": {"command": "platform"}
}"#;

#[derive(serde::Deserialize)]
struct ApiEnvelope<T> {
    ok: bool,
    data: Option<T>,
}

#[derive(serde::Deserialize)]
struct SyncStatusData {
    allowlist: Vec<AllowlistRepo>,
    allowlist_etag: Option<String>,
}

#[test]
fn status_envelope_allowlist_parses_camel_case() {
    let env: ApiEnvelope<SyncStatusData> =
        serde_json::from_str(STATUS_ENVELOPE).expect("status json");
    assert!(env.ok);
    let data = env.data.expect("data");
    assert_eq!(data.allowlist.len(), 3);
    assert_eq!(data.allowlist[0].full_name, "codasignal/foo");
    assert_eq!(
        data.allowlist_etag.as_deref(),
        Some("codasignal/foo|org/cli|other/cli")
    );

    // AC-14: allowlisted label matches.
    assert_eq!(
        classify_repo("CodaSignal/foo", &data.allowlist),
        MatchOutcome::Allowed
    );
    // AC-15: personal / unbound label skips.
    assert_eq!(
        classify_repo("my-dotfiles", &data.allowlist),
        MatchOutcome::SkippedNotInOrg
    );
    // AC-17: ambiguous basename.
    assert_eq!(
        classify_repo("cli", &data.allowlist),
        MatchOutcome::SkippedAmbiguous
    );
    assert_eq!(
        classify_repo("org/cli", &data.allowlist),
        MatchOutcome::Allowed
    );
}

#[test]
fn gated_import_envelope_parses_repo_not_allowed() {
    let env: ApiEnvelope<ImportResponse> = serde_json::from_str(GATED_IMPORT).expect("import json");
    assert!(env.ok);
    let data = env.data.expect("data");
    assert_eq!(data.results.len(), 1);
    assert_eq!(data.results[0].status, ImportStatus::RepoNotAllowed);
}

#[test]
fn mint_and_workspace_list_shapes() {
    #[derive(serde::Deserialize)]
    struct Body {
        workspaces: Vec<View>,
    }
    #[derive(serde::Deserialize)]
    struct View {
        workspace: Inner,
    }
    #[derive(serde::Deserialize)]
    struct Inner {
        id: String,
        name: String,
    }

    let mint = r#"{
      "secret": "cmk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "keyPrefix": "cmk_aaaa",
      "personalWorkspaceId": "ws-personal",
      "apiUrl": "http://127.0.0.1:8787"
    }"#;
    let parsed: comemory::sync::client::MintDeviceKeyResponse =
        serde_json::from_str(mint).expect("mint");
    assert_eq!(parsed.personal_workspace_id, "ws-personal");

    let list = r#"{
      "workspaces": [
        {
          "workspace": {"id": "ws-personal", "name": "Personal", "createdAt": "2026-09-06T00:00:00.000Z"},
          "corpus": {"memoriesEnabled": true, "codeEnabled": true, "docsEnabled": false}
        }
      ]
    }"#;
    let body: Body = serde_json::from_str(list).expect("list");
    assert_eq!(body.workspaces[0].workspace.id, "ws-personal");
    assert_eq!(body.workspaces[0].workspace.name, "Personal");
}
