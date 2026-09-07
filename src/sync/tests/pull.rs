#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! `run_pull` against a loopback platform that returns a real upsert entry.

use time::OffsetDateTime;

use comemory::config::{Config, Paths};
use comemory::memory::id::{memory_id, sha256_hex};
use comemory::memory::{Kind, MemoryStore};
use comemory::store::connection;
use comemory::store::sync_state;
use comemory::sync::AuthFile;
use comemory::sync::pull;

use crate::test_common::sync_platform_server::{SyncPlatformServer, SyncPlatformState};

fn upsert_wire_entry(body: &str, seq: i64) -> serde_json::Value {
    let id = memory_id(body);
    let content_hash = sha256_hex(body.trim_end().as_bytes());
    let created = OffsetDateTime::now_utc();
    serde_json::json!({
        "seq": seq,
        "op": "upsert",
        "id": id,
        "content_hash": content_hash,
        "at": "2026-09-06T12:00:00Z",
        "author": "peer-device",
        "record": {
            "frontmatter": {
                "id": id,
                "kind": "note",
                "repo": "codasignal/foo",
                "tags": ["sync"],
                "created": created.format(&time::format_description::well_known::Iso8601::DEFAULT)
                    .expect("iso"),
                "quality": 3,
                "schema": 1,
                "content_hash": content_hash,
                "references": {"symbols": [], "files": []},
                "relations": {
                    "supersedes": [],
                    "conflicts_with": [],
                    "derived_from": []
                }
            },
            "body": body,
            "vector": null
        }
    })
}

fn seed_auth(paths: &Paths, api_url: &str, secret: &str, workspace: &str) {
    AuthFile {
        secret: secret.into(),
        key_prefix: "cmk_bbbb".into(),
        personal_workspace_id: workspace.into(),
        api_url: api_url.into(),
        device_name: "test".into(),
        email: None,
    }
    .save(paths)
    .expect("auth");
}

#[test]
fn pull_applies_remote_upsert_and_advances_cursor() {
    let body = "remote upsert pulled into a fresh local store";
    let platform = SyncPlatformState {
        head_seq: 7,
        changes: serde_json::json!([upsert_wire_entry(body, 7)]),
        ..Default::default()
    };
    let server = SyncPlatformServer::start(platform);
    let secret = server.snapshot().secret;

    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let cfg = Config::defaults();
    let workspace = "ws-org";
    seed_auth(&paths, &server.base, &secret, workspace);

    let auth = AuthFile::load(&paths).expect("load").expect("auth");
    let pull_stats = pull::run_pull(&paths, &cfg, &mut conn, &auth, workspace, 100).expect("pull");
    assert_eq!(pull_stats.pulled, 1);
    assert_eq!(pull_stats.last_pulled_seq, 7);

    let row = sync_state::get(&conn, workspace)
        .expect("get")
        .expect("row");
    assert_eq!(row.pulled_seq, 7);
    assert!(row.last_sync_at.is_some());

    let store = MemoryStore::new(paths);
    let rec = store.load(&memory_id(body)).expect("memory");
    assert_eq!(rec.body, body);
    assert_eq!(rec.frontmatter.kind, Kind::Note);
}

#[test]
fn empty_remote_changes_still_records_head() {
    let platform = SyncPlatformState {
        head_seq: 42,
        changes: serde_json::json!([]),
        ..Default::default()
    };
    let server = SyncPlatformServer::start(platform);
    let secret = server.snapshot().secret;

    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let cfg = Config::defaults();
    let workspace = "ws-org";
    seed_auth(&paths, &server.base, &secret, workspace);

    let auth = AuthFile::load(&paths).expect("load").expect("auth");
    let pull_stats = pull::run_pull(&paths, &cfg, &mut conn, &auth, workspace, 50).expect("pull");
    assert_eq!(pull_stats.pulled, 0);
    assert_eq!(pull_stats.last_pulled_seq, 42);
}
