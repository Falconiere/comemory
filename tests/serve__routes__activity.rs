#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! What a real loopback `comemory serve` records in `activity_log`: the
//! `User-Agent` it reports as the caller's `actor`, and the silence a
//! `--read-only` server keeps.
//!
//! Requests go out through this file's own `reqwest` client rather than the
//! shared harness's, because the header under test is one the harness does
//! not set.

#[path = "common/serve_bin.rs"]
mod serve_bin;

use assert_cmd::Command as AssertCommand;
use comemory::store::activity::{ActivityFilter, ActivityRow, list};
use comemory::store::connection;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use serve_bin::ServeHome;
use tempfile::TempDir;

/// Every row recorded into the data dir at `data_dir`, newest first.
fn rows_at(data_dir: &std::path::Path) -> Vec<ActivityRow> {
    let conn = connection::open(data_dir.join("comemory.db")).expect("open db");
    list(&conn, &ActivityFilter::default(), 0, 0)
        .expect("list activity")
        .0
}

/// Every row the server recorded, newest first.
fn rows(home: &ServeHome) -> Vec<ActivityRow> {
    rows_at(&home.data_dir())
}

/// `POST <path>` (relative to `/api/v1`) with an explicit `User-Agent`,
/// returning the whole envelope.
fn post_as(home: &ServeHome, agent: &str, path: &str, body: &Value) -> Value {
    let response = Client::new()
        .post(home.url(path))
        .header("X-Comemory-Token", home.token())
        .header(reqwest::header::USER_AGENT, agent)
        .json(body)
        .send()
        .expect("request");
    response.json().expect("an enveloped body")
}

#[test]
fn an_http_save_records_the_callers_user_agent_as_its_actor() {
    let home = ServeHome::new();

    let saved = post_as(
        &home,
        "acme-console/3",
        "/memories",
        &json!({
            "body": "pgbouncer in transaction mode fixes pool exhaustion",
            "repo": "demo",
            "kind": "decision",
        }),
    );
    assert_eq!(saved["ok"], true, "save failed: {saved}");

    let recorded = rows(&home);
    let save_row = recorded
        .iter()
        .find(|r| r.command == "save")
        .expect("a save row");
    assert_eq!(save_row.source, "http");
    assert_eq!(save_row.actor.as_deref(), Some("acme-console/3"));
    assert_eq!(save_row.repo.as_deref(), Some("demo"));
}

#[test]
fn an_http_search_records_its_own_row_with_the_same_actor() {
    let home = ServeHome::new();
    post_as(
        &home,
        "acme-console/3",
        "/memories",
        &json!({"body": "pgbouncer pool exhaustion", "repo": "demo", "kind": "note"}),
    );
    let searched = post_as(
        &home,
        "acme-console/3",
        "/memories/search",
        &json!({"query": "pgbouncer"}),
    );
    assert_eq!(searched["ok"], true, "search failed: {searched}");

    let recorded = rows(&home);
    let search_row = recorded
        .iter()
        .find(|r| r.command == "search")
        .expect("a search row");
    assert_eq!(search_row.source, "http");
    assert_eq!(search_row.actor.as_deref(), Some("acme-console/3"));
    let summary: Value =
        serde_json::from_str(search_row.summary.as_deref().expect("a summary")).unwrap();
    assert_eq!(summary["query"], "pgbouncer");
}

#[test]
fn a_request_without_a_user_agent_records_no_actor() {
    let home = ServeHome::new();
    let saved = home.post(
        "/memories",
        &json!({"body": "no agent declared here", "repo": "demo", "kind": "note"}),
    );
    assert!(saved.get("id").is_some(), "save failed: {saved}");

    let recorded = rows(&home);
    let save_row = recorded
        .iter()
        .find(|r| r.command == "save")
        .expect("a save row");
    assert_eq!(
        save_row.actor, None,
        "an actor is never inferred when the caller declared none"
    );
}

#[test]
fn a_read_only_server_serves_searches_and_records_nothing() {
    // Seeded by the real CLI so the read-only server below starts on a data
    // dir that already holds a memory and its own recorded rows.
    let root = TempDir::new().expect("tempdir");
    let data_dir = root.path().join(".comemory");
    AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", &data_dir)
        .args(["save", "pgbouncer pool exhaustion", "--repo", "demo"])
        .assert()
        .success();
    let before = rows_at(&data_dir).len();
    assert_eq!(before, 1, "the seeding save is on record");

    let home = ServeHome::spawn_in(root, &["--read-only"], &[]);
    let searched = post_as(
        &home,
        "acme-console/3",
        "/memories/search",
        &json!({"query": "pgbouncer"}),
    );
    assert_eq!(searched["ok"], true, "a read-only search still answers");

    assert_eq!(
        rows(&home).len(),
        before,
        "a read-only server writes nothing to the store, telemetry included"
    );
}

#[test]
fn the_snapshot_route_pages_filters_and_rolls_up_what_the_server_recorded() {
    let home = ServeHome::new();
    post_as(
        &home,
        "acme-console/3",
        "/memories",
        &json!({"body": "pgbouncer pool exhaustion", "repo": "demo", "kind": "note"}),
    );
    post_as(
        &home,
        "acme-console/3",
        "/memories/search",
        &json!({"query": "pgbouncer"}),
    );

    let all = home.get("/activity");
    assert_eq!(all["total"].as_u64().expect("a total"), 2);
    assert!(all["cursor"].as_i64().expect("a cursor") > 0);
    let commands: Vec<&str> = all["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["command"].as_str().expect("a command"))
        .collect();
    assert_eq!(commands, ["search", "save"], "newest first");
    assert_eq!(all["items"][0]["actor"], "acme-console/3");
    assert_eq!(
        all["items"][0]["summary"]["query"], "pgbouncer",
        "the summary arrives as JSON, not as a string"
    );

    let saves = home.get_q("/activity", &[("command", "save")]);
    assert_eq!(saves["total"].as_u64().expect("a total"), 1);
    assert_eq!(saves["items"].as_array().expect("items").len(), 1);
    let rollups = saves["rollups"].as_array().expect("rollups");
    assert_eq!(rollups.len(), 1);
    assert_eq!(rollups[0]["command"], "save");
    assert_eq!(rollups[0]["runs"], 1);
    assert_eq!(rollups[0]["errors"], 0);
}

#[test]
fn the_snapshot_route_answers_an_untouched_server_with_an_empty_feed() {
    // A server opens `comemory.db` at startup, so the no-database invariant
    // is proven at the core (`src/domains/maintenance/tests/activity.rs`).
    // What the route owes here is an empty, well-formed page.
    let home = ServeHome::new();

    let out = home.get("/activity");
    assert_eq!(out["items"].as_array().expect("items").len(), 0);
    assert_eq!(out["total"], 0);
    assert_eq!(out["rollups"].as_array().expect("rollups").len(), 0);
    assert_eq!(out["cursor"], 0, "nothing recorded yet, so no cursor");
}

#[test]
fn the_route_table_lists_the_feed_as_a_read() {
    // `GET /commands` maps CLI subcommands onto routes, and the feed has no
    // CLI counterpart (spec Non-Goal 1) — so the inventory that matters is
    // the route table itself, the one the read-only gate consults.
    let entries = comemory::serve::routes::table();
    let feed: Vec<_> = entries
        .iter()
        .filter(|e| e.path.starts_with("/activity"))
        .collect();
    assert_eq!(
        feed.iter()
            .map(|e| (e.method, e.path, e.command))
            .collect::<Vec<_>>(),
        [
            ("GET", "/activity", "activity"),
            ("GET", "/activity/events", "activity.events"),
        ],
        "the snapshot and its stream, both inventoried"
    );
    assert!(
        feed.iter().all(|e| !e.mutating),
        "reading the feed writes nothing, so a read-only server serves both"
    );
}
