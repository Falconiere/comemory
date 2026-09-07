#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! In-process coverage of `GET|POST /api/v1/sync/*` through the real router.

use comemory::api::sync::SyncOp;
use comemory::memory::id::{memory_id, sha256_hex};
use comemory::memory::{Kind, MemoryStore};
use serde_json::json;
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::test_common::serve_state;

#[test]
fn table_entries_has_three_routes() {
    assert_eq!(comemory::serve::routes::sync::table_entries().len(), 3);
}

#[tokio::test]
async fn changes_and_manifest_empty_corpus() {
    let session = serve_state::session(false);

    let changes = serve_state::send(
        &session,
        "GET",
        "/api/v1/sync/changes?since=0&limit=50",
        None,
    )
    .await;
    assert_eq!(changes.status, 200, "body: {}", changes.text);
    assert_eq!(changes.json["meta"]["command"], "sync.changes");
    assert!(
        changes.json["data"]["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(changes.json["data"]["head_seq"], 0);

    let manifest = serve_state::send(&session, "GET", "/api/v1/sync/manifest", None).await;
    assert_eq!(manifest.status, 200, "body: {}", manifest.text);
    assert_eq!(manifest.json["meta"]["command"], "sync.manifest");
    assert_eq!(
        manifest.json["data"]["buckets"]
            .as_array()
            .expect("buckets")
            .len(),
        256
    );
    assert_eq!(manifest.json["data"]["head_seq"], 0);
}

#[tokio::test]
async fn import_upsert_then_changes_returns_entry() {
    let session = serve_state::session(false);
    let body = "serve sync import creates a memory visible on changes";
    let id = memory_id(body);
    let content_hash = sha256_hex(body.trim_end().as_bytes());
    let created = OffsetDateTime::now_utc()
        .format(&Iso8601::DEFAULT)
        .expect("iso");

    let imported = serve_state::send(
        &session,
        "POST",
        "/api/v1/sync/import",
        Some(json!({
            "cursor": 0,
            "entries": [{
                "op": "upsert",
                "id": id,
                "content_hash": content_hash,
                "at": "2026-09-06T12:00:00Z",
                "record": {
                    "frontmatter": {
                        "id": id,
                        "kind": "note",
                        "repo": "demo",
                        "tags": ["sync"],
                        "created": created,
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
                    "body": body
                }
            }]
        })),
    )
    .await;
    assert_eq!(imported.status, 200, "body: {}", imported.text);
    assert_eq!(imported.json["meta"]["command"], "sync.import");
    assert_eq!(imported.json["data"]["results"][0]["status"], "accepted");
    assert!(
        imported.json["data"]["results"][0]["seq"]
            .as_i64()
            .is_some()
    );

    let paths = comemory::config::Paths::new(session.home.path());
    let store = MemoryStore::new(paths);
    let rec = store.load(&id).expect("live");
    assert_eq!(rec.frontmatter.kind, Kind::Note);
    assert_eq!(rec.body, body);

    let changes = serve_state::send(
        &session,
        "GET",
        "/api/v1/sync/changes?since=0&limit=10",
        None,
    )
    .await;
    assert_eq!(changes.status, 200, "body: {}", changes.text);
    let entries = changes.json["data"]["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["op"], "upsert");
    assert_eq!(entries[0]["id"], id);
    assert_eq!(entries[0]["op"].as_str(), Some(SyncOp::Upsert.as_str()));
}

#[tokio::test]
async fn import_respects_author_header() {
    let session = serve_state::session(false);
    let body = "author header stamps the imported memory";
    let id = memory_id(body);
    let content_hash = sha256_hex(body.trim_end().as_bytes());
    let created = OffsetDateTime::now_utc()
        .format(&Iso8601::DEFAULT)
        .expect("iso");
    let token = session.token.clone();

    let resp = serve_state::send_headers(
        &session,
        "POST",
        "/api/v1/sync/import",
        &[
            ("Host", "127.0.0.1"),
            ("X-Comemory-Token", &token),
            ("X-Comemory-Author", "device-alice"),
        ],
        Some(json!({
            "cursor": 0,
            "entries": [{
                "op": "upsert",
                "id": id,
                "content_hash": content_hash,
                "at": "2026-09-06T12:00:00Z",
                "record": {
                    "frontmatter": {
                        "id": id,
                        "kind": "note",
                        "repo": "demo",
                        "tags": [],
                        "created": created,
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
                    "body": body
                }
            }]
        })),
    )
    .await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);

    let paths = comemory::config::Paths::new(session.home.path());
    let rec = MemoryStore::new(paths).load(&id).expect("load");
    assert_eq!(rec.frontmatter.author, "device-alice");
}
