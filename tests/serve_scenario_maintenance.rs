#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Maintenance journey over `/api/v1` — the HTTP twin of
//! `tests/cli_scenario_maintenance.rs`: save a near-dup pair plus a
//! low-value memory, doctor the low-value row into prune-eligibility →
//! `GET /consolidate` clusters the pair → `GET /prune` flags the low-value
//! row → `POST /prune` (unconfirmed 400, confirmed applies) → `GET /memories`
//! drops the pruned id and keeps the keeper → `POST /rebuild` (job) →
//! search still finds the keeper → `POST /gc` reports removal counts,
//! against a real `comemory serve`.

#[path = "common/serve_bin.rs"]
mod serve_bin;

use serde_json::json;
use serve_bin::ServeHome;

/// Body A for the near-duplicate pair. Measured in
/// `tests/common/cli_save_support.rs`: Hamming(A, B) = 5.
const DUP_BODY_A: &str = "postgres connection pool exhausts under load spikes raise \
     max_connections to fifty and add pgbouncer in transaction mode for the api workers \
     during peak traffic hours";
/// Body B: A with one word changed (`fifty` → `eighty`).
const DUP_BODY_B: &str = "postgres connection pool exhausts under load spikes raise \
     max_connections to eighty and add pgbouncer in transaction mode for the api workers \
     during peak traffic hours";

/// `POST /memories` over HTTP, returning the saved id.
fn save_id(srv: &ServeHome, body: &str) -> String {
    srv.post("/memories", &json!({ "body": body, "kind": "note" }))["id"]
        .as_str()
        .expect("id")
        .to_string()
}

/// Doctor a live row into prune-eligibility exactly like
/// `cli_scenario_maintenance.rs`'s `make_prune_eligible`: open the server's
/// own `comemory.db` from a second, short-lived connection and run one
/// UPDATE, then drop that connection immediately. The server keeps its own
/// long-lived connection open throughout — SQLite's own locking handles the
/// concurrent writer.
fn make_prune_eligible(srv: &ServeHome, id: &str) {
    let db = srv.data_dir().join("comemory.db");
    let conn = comemory::store::connection::open(db).expect("open mirror");
    conn.execute(
        "UPDATE memories SET quality = 2, last_accessed = '2025-01-01T00:00:00Z' WHERE id = ?1",
        [id],
    )
    .expect("doctor row");
}

#[test]
fn consolidate_prune_rebuild_gc_over_http() {
    let srv = ServeHome::new();
    let keeper = save_id(&srv, DUP_BODY_A);
    let _dup = save_id(&srv, DUP_BODY_B);
    let low = save_id(
        &srv,
        "unrelated prune candidate about lighthouse inspection logs",
    );
    make_prune_eligible(&srv, &low);

    let clusters = srv.get("/consolidate");
    assert!(
        clusters["clusters"]["total"].as_u64().expect("total") >= 1,
        "near-dup pair must cluster: {clusters}"
    );

    let dry = srv.get("/prune");
    let low_total = dry["low_value_memories"]["total"].as_u64().expect("total");
    assert!(
        low_total >= 1,
        "prune dry-run must flag the low-value row: {dry}"
    );

    let (status, body) = srv.post_raw("/prune", &json!({ "apply": true }));
    assert_eq!(status, 400, "unconfirmed apply must be rejected: {body}");
    assert_eq!(body["error"]["code"], "confirmation_required");

    // This IS `prune --apply`: `apply` rides in the body verbatim (a `POST`
    // carries no forced override the way `GET /prune` does), `confirm` is
    // the HTTP-only gate.
    srv.post("/prune", &json!({ "apply": true, "confirm": true }));

    let listed = srv.get("/memories");
    let ids: Vec<&str> = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(
        !ids.contains(&low.as_str()),
        "applied prune must drop {low}: {listed}"
    );
    assert!(
        ids.contains(&keeper.as_str()),
        "keeper must survive prune: {listed}"
    );

    let rebuilt = srv.job("/rebuild", &json!({ "confirm": true }));
    assert!(
        rebuilt.is_null(),
        "rebuild emits nothing on success: {rebuilt}"
    );

    // "fifty" is unique to DUP_BODY_A; "pgbouncer" is shared and the
    // near-dup collapse may keep only B in a mixed query.
    let search = srv.get_q("/memories/search", &[("query", "fifty")]);
    let hits = search["hits"].as_array().expect("hits");
    assert!(
        hits.iter()
            .any(|h| h["memory_id"].as_str() == Some(keeper.as_str())),
        "rebuild must still search the keeper: {search}"
    );

    let gc = srv.post("/gc", &json!({ "confirm": true }));
    assert!(gc.get("removed").is_some(), "gc reports removed: {gc}");
    assert!(gc.get("log_rows").is_some(), "gc reports log_rows: {gc}");
    assert!(
        gc.get("event_rows").is_some(),
        "gc reports event_rows: {gc}"
    );
}
