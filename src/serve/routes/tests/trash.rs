#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `GET /api/v1/trash` and `POST /api/v1/trash/{id}/restore` driven through
//! the real router (`tests/common/serve_state.rs`) — console-api spec AC-17
//! at the transport layer, plus the read/mutate split: the listing stays
//! available on a `--read-only` server, the restore does not.

use comemory::memory::{Kind, id};
use serde_json::{Value, json};

use crate::test_common::serve_state::{self, Resp, Session};

/// The `data` object of a `{ok, data, meta}` envelope, or a panic naming the
/// whole body.
fn data(resp: &Resp) -> &Value {
    resp.json
        .get("data")
        .unwrap_or_else(|| panic!("no data in {}", resp.text))
}

/// Seed one memory and soft-delete it through the real routes, returning its
/// id.
async fn save_and_delete(session: &Session, body: &str) -> String {
    serve_state::save(session, body, Kind::Note, "app");
    let memory_id = id::memory_id(body);
    let deleted = serve_state::send(
        session,
        "DELETE",
        &format!("/api/v1/memories/{memory_id}?confirm=true"),
        None,
    )
    .await;
    assert_eq!(deleted.status.as_u16(), 200, "body: {}", deleted.text);
    memory_id
}

#[tokio::test]
async fn ac17_the_listing_reports_the_full_retention_window_on_the_day_of_deletion() {
    let session = serve_state::session(false);
    let memory_id = save_and_delete(&session, "the nightly job was retired in march").await;

    let resp = serve_state::send(&session, "GET", "/api/v1/trash", None).await;

    assert_eq!(resp.status.as_u16(), 200, "body: {}", resp.text);
    let items = data(&resp)["items"]
        .as_array()
        .unwrap_or_else(|| panic!("no items in {}", resp.text));
    assert_eq!(items.len(), 1, "got {items:?}");
    assert_eq!(items[0]["id"], json!(memory_id));
    assert_eq!(
        items[0]["title"],
        json!("the nightly job was retired in march")
    );
    assert_eq!(items[0]["kind"], json!("note"));
    assert_eq!(
        items[0]["days_until_gc"],
        json!(30),
        "the default prune.trash_retention_days is 30"
    );
    assert!(
        items[0]["path"]
            .as_str()
            .is_some_and(|p| p.contains(".trash")),
        "path points into .trash/: {}",
        items[0]
    );
}

#[tokio::test]
async fn the_listing_pages_and_reports_a_total() {
    let session = serve_state::session(false);
    for n in 0..3 {
        save_and_delete(&session, &format!("trash listing row {n}")).await;
    }

    let resp = serve_state::send(&session, "GET", "/api/v1/trash?limit=2&offset=0", None).await;

    assert_eq!(resp.status.as_u16(), 200, "body: {}", resp.text);
    assert_eq!(data(&resp)["total"], json!(3));
    assert_eq!(data(&resp)["has_more"], json!(true));
    let items = data(&resp)["items"]
        .as_array()
        .unwrap_or_else(|| panic!("no items in {}", resp.text));
    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn the_trash_restore_route_brings_a_memory_back() {
    let session = serve_state::session(false);
    let memory_id = save_and_delete(&session, "restored from the trash resource").await;

    let restored = serve_state::send(
        &session,
        "POST",
        &format!("/api/v1/trash/{memory_id}/restore"),
        None,
    )
    .await;
    assert_eq!(restored.status.as_u16(), 200, "body: {}", restored.text);
    assert_eq!(data(&restored)["id"], json!(memory_id));

    let empty = serve_state::send(&session, "GET", "/api/v1/trash", None).await;
    assert_eq!(data(&empty)["items"], json!([]), "the trash is empty again");

    let back = serve_state::send(
        &session,
        "GET",
        &format!("/api/v1/memories/{memory_id}"),
        None,
    )
    .await;
    assert_eq!(back.status.as_u16(), 200, "body: {}", back.text);
}

#[tokio::test]
async fn restoring_an_unknown_id_is_404_not_found() {
    let session = serve_state::session(false);

    let resp = serve_state::send(&session, "POST", "/api/v1/trash/deadbeef/restore", None).await;

    assert_eq!(resp.status.as_u16(), 404, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], json!("not_found"));
}

#[tokio::test]
async fn the_listing_reads_on_a_read_only_server_but_the_restore_does_not() {
    let session = serve_state::session(true);

    let listed = serve_state::send(&session, "GET", "/api/v1/trash", None).await;
    assert_eq!(listed.status.as_u16(), 200, "body: {}", listed.text);
    assert_eq!(data(&listed)["items"], json!([]));

    let restored =
        serve_state::send(&session, "POST", "/api/v1/trash/deadbeef/restore", None).await;
    assert_eq!(restored.status.as_u16(), 405, "body: {}", restored.text);
    assert_eq!(restored.json["error"]["code"], json!("read_only"));
}
