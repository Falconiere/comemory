#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `PATCH /api/v1/memories/{id}`, `POST /api/v1/memories/{id}/restore` and
//! `POST /api/v1/memories/{id}/references/refresh` driven through the real
//! router (`tests/common/serve_state.rs`) — console-api spec AC-6 and AC-7
//! at the transport layer, plus the read-only gate every mutating route
//! carries.
//!
//! Ids are derived with `memory::id::memory_id` (the same content hash the
//! save path uses) rather than scraped from a response, so each test names
//! the memory it seeded.

use comemory::memory::{Kind, id};
use serde_json::{Value, json};

use crate::test_common::serve_state;

/// The `data` object of a `{ok, data, meta}` envelope, or a panic naming the
/// whole body — a failing route is far easier to debug from its own error.
fn data(resp: &serve_state::Resp) -> &Value {
    resp.json
        .get("data")
        .unwrap_or_else(|| panic!("no data in {}", resp.text))
}

#[tokio::test]
async fn ac6_patching_tags_keeps_the_id_and_shows_the_new_tags() {
    let session = serve_state::session(false);
    let body = "the write path fsyncs before the rename";
    serve_state::save(&session, body, Kind::Convention, "app");
    let memory_id = id::memory_id(body);

    let resp = serve_state::send(
        &session,
        "PATCH",
        &format!("/api/v1/memories/{memory_id}"),
        Some(json!({ "tags": ["durability", "io"] })),
    )
    .await;

    assert_eq!(resp.status.as_u16(), 200, "body: {}", resp.text);
    assert_eq!(data(&resp)["id"], json!(memory_id));
    assert_eq!(data(&resp)["superseded"], Value::Null);
    assert_eq!(data(&resp)["changed"], json!(["tags"]));

    let shown = serve_state::send(
        &session,
        "GET",
        &format!("/api/v1/memories/{memory_id}"),
        None,
    )
    .await;
    let mut tags: Vec<String> = shown.json["data"]["tags"]
        .as_array()
        .unwrap_or_else(|| panic!("no tags in {}", shown.text))
        .iter()
        .filter_map(|t| t.as_str().map(str::to_string))
        .collect();
    tags.sort();
    assert_eq!(tags, vec!["durability".to_string(), "io".to_string()]);
}

#[tokio::test]
async fn ac6_patching_the_body_mints_a_new_id_that_supersedes_the_old() {
    let session = serve_state::session(false);
    let body = "retries use a fixed backoff";
    serve_state::save(&session, body, Kind::Decision, "app");
    let old_id = id::memory_id(body);

    let resp = serve_state::send(
        &session,
        "PATCH",
        &format!("/api/v1/memories/{old_id}"),
        Some(json!({ "body": "retries use an exponential backoff" })),
    )
    .await;

    assert_eq!(resp.status.as_u16(), 200, "body: {}", resp.text);
    let new_id = data(&resp)["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no id in {}", resp.text))
        .to_string();
    assert_ne!(new_id, old_id);
    assert_eq!(data(&resp)["superseded"], json!(old_id));

    // `api::show` reports the supersede edge on the OLD id.
    let old = serve_state::send(&session, "GET", &format!("/api/v1/memories/{old_id}"), None).await;
    assert_eq!(old.json["data"]["superseded_by"], json!(new_id));

    let new = serve_state::send(&session, "GET", &format!("/api/v1/memories/{new_id}"), None).await;
    assert_eq!(new.status.as_u16(), 200, "body: {}", new.text);
    assert_eq!(
        new.json["data"]["body"],
        json!("retries use an exponential backoff")
    );
}

#[tokio::test]
async fn patching_an_unknown_id_is_404_not_found() {
    let session = serve_state::session(false);

    let resp = serve_state::send(
        &session,
        "PATCH",
        "/api/v1/memories/deadbeef",
        Some(json!({ "quality": 5 })),
    )
    .await;

    assert_eq!(resp.status.as_u16(), 404, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], json!("not_found"));
}

#[tokio::test]
async fn ac7_delete_then_restore_through_the_memories_route() {
    let session = serve_state::session(false);
    let body = "the outbox table is drained by a single worker";
    serve_state::save(&session, body, Kind::Pattern, "app");
    let memory_id = id::memory_id(body);

    let deleted = serve_state::send(
        &session,
        "DELETE",
        &format!("/api/v1/memories/{memory_id}?confirm=true"),
        None,
    )
    .await;
    assert_eq!(deleted.status.as_u16(), 200, "body: {}", deleted.text);

    let gone = serve_state::send(
        &session,
        "GET",
        &format!("/api/v1/memories/{memory_id}"),
        None,
    )
    .await;
    assert_eq!(
        gone.status.as_u16(),
        404,
        "soft-deleted memories are hidden"
    );

    let restored = serve_state::send(
        &session,
        "POST",
        &format!("/api/v1/memories/{memory_id}/restore"),
        None,
    )
    .await;
    assert_eq!(restored.status.as_u16(), 200, "body: {}", restored.text);
    assert_eq!(data(&restored)["id"], json!(memory_id));

    let back = serve_state::send(
        &session,
        "GET",
        &format!("/api/v1/memories/{memory_id}"),
        None,
    )
    .await;
    assert_eq!(back.status.as_u16(), 200, "body: {}", back.text);
    assert_eq!(back.json["data"]["kind"], json!("pattern"));
}

#[tokio::test]
async fn restoring_a_live_memory_is_400_bad_request() {
    let session = serve_state::session(false);
    let body = "this memory was never deleted";
    serve_state::save(&session, body, Kind::Note, "app");
    let memory_id = id::memory_id(body);

    let resp = serve_state::send(
        &session,
        "POST",
        &format!("/api/v1/memories/{memory_id}/restore"),
        None,
    )
    .await;

    assert_eq!(resp.status.as_u16(), 400, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], json!("bad_request"));
}

#[tokio::test]
async fn references_refresh_answers_with_the_reclassified_code_refs() {
    let session = serve_state::session(false);
    // A backtick-fenced mention is mined into a `references_file` edge by
    // `cross_link`, so this memory has a code ref without any `--ref-file`.
    let body = "the retry budget lives in `app:src/retry.rs`";
    serve_state::save(&session, body, Kind::Note, "app");
    let memory_id = id::memory_id(body);

    let resp = serve_state::send(
        &session,
        "POST",
        &format!("/api/v1/memories/{memory_id}/references/refresh"),
        None,
    )
    .await;

    assert_eq!(resp.status.as_u16(), 200, "body: {}", resp.text);
    assert_eq!(data(&resp)["id"], json!(memory_id));
    // Nothing to re-pin: a body mention carries no frontmatter anchor, and
    // the repo was never indexed, so the ref still surfaces in `code_refs`.
    assert_eq!(data(&resp)["refreshed"], json!(0));
    let refs = data(&resp)["code_refs"]
        .as_array()
        .unwrap_or_else(|| panic!("no code_refs in {}", resp.text));
    assert_eq!(refs.len(), 1, "got {refs:?}");
    assert_eq!(refs[0]["anchor"], json!("app:src/retry.rs"));
}

#[tokio::test]
async fn every_edit_route_is_405_read_only_on_a_read_only_server() {
    let session = serve_state::session(true);

    for (method, path, body) in [
        (
            "PATCH",
            "/api/v1/memories/deadbeef",
            Some(json!({ "quality": 5 })),
        ),
        ("POST", "/api/v1/memories/deadbeef/restore", None),
        ("POST", "/api/v1/memories/deadbeef/references/refresh", None),
    ] {
        let resp = serve_state::send(&session, method, path, body).await;
        assert_eq!(
            resp.status.as_u16(),
            405,
            "{method} {path} body: {}",
            resp.text
        );
        assert_eq!(resp.json["error"]["code"], json!("read_only"));
    }
}
