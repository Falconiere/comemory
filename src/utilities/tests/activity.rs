#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirrors `src/utilities/activity.rs` against a real migrated `comemory.db`:
//! what an origin records, what a failed run records, and the four ways the
//! writer is allowed to give up without touching the caller's result.

use std::time::Duration;

use comemory::config::Config;
use comemory::prelude::Error;
use comemory::store::activity::{ActivityFilter, list};
use comemory::store::connection;
use comemory::utilities::activity::{Origin, Outcome, command, record, source};
use serde_json::json;
use tempfile::tempdir;

fn cfg() -> Config {
    Config::defaults()
}

fn cfg_with(enabled: bool, summaries: bool) -> Config {
    let mut c = Config::defaults();
    c.activity.enabled = enabled;
    c.activity.summaries = summaries;
    c
}

#[test]
fn a_cli_run_records_its_command_source_summary_and_repo() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    let cfg = cfg();

    record(
        &conn,
        &Origin::cli(&cfg),
        command::SAVE,
        Duration::from_millis(37),
        &Outcome::Ok(&json!({"id": "a1b2c3d4", "title": "A lesson"})),
        Some("comemory"),
    );

    let (rows, total) = list(&conn, &ActivityFilter::default(), 0, 0).unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows[0].command, "save");
    assert_eq!(rows[0].source, source::CLI);
    assert_eq!(rows[0].repo.as_deref(), Some("comemory"));
    assert_eq!(rows[0].duration_ms, 37);
    assert!(rows[0].ok);
    assert_eq!(rows[0].error_code, None);
    assert_eq!(
        rows[0].summary.as_deref(),
        Some(r#"{"id":"a1b2c3d4","title":"A lesson"}"#)
    );
    assert_eq!(rows[0].actor, None, "no COMEMORY_ACTOR, so no actor");
}

#[test]
fn an_http_actor_is_trimmed_and_bounded_and_an_mcp_client_rides_through() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    let cfg = cfg();
    let long = "x".repeat(400);

    record(
        &conn,
        &Origin::http(&cfg, Some(&format!("  {long}  "))),
        command::SEARCH,
        Duration::from_millis(2),
        &Outcome::Ok(&json!({"hits": 0})),
        None,
    );
    record(
        &conn,
        &Origin::mcp(&cfg, Some("claude-code/2.1.0")),
        command::FIND,
        Duration::from_millis(5),
        &Outcome::Ok(&json!({"hits": 3})),
        None,
    );

    let (rows, _) = list(&conn, &ActivityFilter::default(), 0, 0).unwrap();
    let mcp = rows.iter().find(|r| r.source == source::MCP).unwrap();
    assert_eq!(mcp.actor.as_deref(), Some("claude-code/2.1.0"));

    let http = rows.iter().find(|r| r.source == source::HTTP).unwrap();
    let actor = http.actor.as_deref().unwrap();
    assert_eq!(actor.chars().count(), 120, "bounded to 120 chars");
    assert!(actor.starts_with('x'), "padding trimmed before bounding");
}

#[test]
fn a_failed_run_records_its_error_slug_and_drops_the_summary() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    let cfg = cfg();

    let failure = Error::BadRequest("save needs a repo scope".into());
    record(
        &conn,
        &Origin::cli(&cfg),
        command::SAVE,
        Duration::from_millis(1),
        &Outcome::Failed(&failure),
        None,
    );

    let (rows, _) = list(&conn, &ActivityFilter::default(), 0, 0).unwrap();
    assert!(!rows[0].ok);
    assert_eq!(rows[0].error_code.as_deref(), Some("bad_request"));
    assert_eq!(rows[0].summary, None, "a failure carries no summary");
}

#[test]
fn a_disabled_feed_and_a_read_only_origin_record_nothing() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();

    let off = cfg_with(false, true);
    record(
        &conn,
        &Origin::cli(&off),
        command::SAVE,
        Duration::from_millis(1),
        &Outcome::Ok(&json!({"id": "a1b2c3d4"})),
        None,
    );

    let on = cfg();
    record(
        &conn,
        &Origin::http(&on, None).read_only(),
        command::SEARCH,
        Duration::from_millis(1),
        &Outcome::Ok(&json!({"hits": 1})),
        None,
    );

    let (_, total) = list(&conn, &ActivityFilter::default(), 0, 0).unwrap();
    assert_eq!(total, 0, "neither origin may write a row");
}

#[test]
fn summaries_off_still_records_the_run_without_its_detail() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    let cfg = cfg_with(true, false);

    record(
        &conn,
        &Origin::cli(&cfg),
        command::FIND,
        Duration::from_millis(9),
        &Outcome::Ok(&json!({"query": "a secret phrase", "hits": 2})),
        None,
    );

    let (rows, total) = list(&conn, &ActivityFilter::default(), 0, 0).unwrap();
    assert_eq!(total, 1, "the run is still recorded");
    assert_eq!(rows[0].summary, None);
    assert_eq!(rows[0].command, "find");
}

#[test]
fn a_missing_activity_table_does_not_disturb_the_caller() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    conn.execute_batch("DROP TABLE activity_log;").unwrap();
    let cfg = cfg();

    // The whole contract: `record` returns, the process lives, and nothing
    // the caller was doing is affected. A telemetry failure is not a command
    // failure.
    record(
        &conn,
        &Origin::cli(&cfg),
        command::SAVE,
        Duration::from_millis(4),
        &Outcome::Ok(&json!({"id": "a1b2c3d4"})),
        None,
    );

    let still_usable: i64 = conn
        .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    assert_eq!(still_usable, 0, "the connection is still usable afterwards");
}
