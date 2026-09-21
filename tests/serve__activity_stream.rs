#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `GET /api/v1/activity/events` as a client actually consumes it: a live
//! connection to a real loopback server, rows written by a SEPARATE
//! `comemory` process, and the reconnect that resumes above the last id.
//!
//! The SSE framing is read off the wire here rather than with a client crate —
//! the point is that the bytes the server emits are the ones an `EventSource`
//! parses: `event:`, `id:` and `data:` lines terminated by a blank line.

#[path = "common/serve_bin.rs"]
mod serve_bin;

use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

use assert_cmd::Command as AssertCommand;
use serde_json::Value;
use serve_bin::ServeHome;

/// How long a test waits for an event before calling the stream broken.
const DEADLINE: Duration = Duration::from_secs(10);

/// One parsed SSE event.
#[derive(Debug)]
struct SseEvent {
    name: String,
    id: Option<String>,
    data: Value,
}

/// Open the stream and return a reader over its body.
fn open_stream(
    home: &ServeHome,
    query: &str,
    last_event_id: Option<&str>,
) -> BufReader<reqwest::blocking::Response> {
    let mut request = reqwest::blocking::Client::builder()
        .timeout(None)
        .build()
        .expect("client")
        .get(format!("{}{}", home.url("/activity/events"), query))
        .header("X-Comemory-Token", home.token());
    if let Some(id) = last_event_id {
        request = request.header("Last-Event-ID", id);
    }
    let response = request.send().expect("open the stream");
    assert!(
        response.status().is_success(),
        "stream refused: {response:?}"
    );
    BufReader::new(response)
}

/// Read events until `want` of them have arrived or the deadline passes.
fn read_events(reader: &mut BufReader<reqwest::blocking::Response>, want: usize) -> Vec<SseEvent> {
    let started = Instant::now();
    let mut events = Vec::new();
    let mut name = None;
    let mut id = None;
    let mut data: Option<String> = None;
    let mut line = String::new();
    while events.len() < want {
        assert!(
            started.elapsed() < DEADLINE,
            "only {} of {want} events arrived within {DEADLINE:?}",
            events.len()
        );
        line.clear();
        let read = reader.read_line(&mut line).expect("read the stream");
        assert!(read > 0, "the server closed the stream");
        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.strip_prefix("event:") {
            name = Some(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("id:") {
            id = Some(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("data:") {
            data = Some(rest.trim().to_string());
        } else if trimmed.is_empty()
            && let Some(payload) = data.take()
        {
            events.push(SseEvent {
                name: name.take().unwrap_or_default(),
                id: id.take(),
                data: serde_json::from_str(&payload).expect("event data is JSON"),
            });
        }
    }
    events
}

/// Save a memory from a SEPARATE process against the server's data dir.
fn save_elsewhere(home: &ServeHome, body: &str) {
    AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.data_dir())
        .env("COMEMORY_ACTOR", "other-process/1")
        .args(["save", body, "--repo", "demo"])
        .assert()
        .success();
}

#[test]
fn a_connected_client_receives_a_row_another_process_wrote() {
    let home = ServeHome::new();
    let mut stream = open_stream(&home, "", None);

    save_elsewhere(&home, "pgbouncer in transaction mode fixes pool exhaustion");

    let events = read_events(&mut stream, 1);
    let event = &events[0];
    assert_eq!(event.name, "activity");
    assert_eq!(event.data["command"], "save");
    assert_eq!(
        event.data["source"], "cli",
        "a separate CLI process wrote it"
    );
    assert_eq!(event.data["actor"], "other-process/1");
    assert_eq!(event.data["repo"], "demo");

    // The event id is the row id, and the snapshot route reports the same row.
    let streamed_id = event.id.as_deref().expect("an event id");
    let snapshot = home.get("/activity");
    assert_eq!(snapshot["items"][0]["id"].to_string(), streamed_id);
    assert_eq!(snapshot["items"][0]["summary"], event.data["summary"]);
}

#[test]
fn the_stream_starts_at_the_newest_row_and_does_not_replay_history() {
    let home = ServeHome::new();
    save_elsewhere(&home, "an older lesson nobody is streaming for");
    let before = home.get("/activity");
    let cursor = before["cursor"].as_i64().expect("a cursor");
    assert!(cursor > 0);

    let mut stream = open_stream(&home, "", None);
    save_elsewhere(&home, "the lesson the stream is open for");

    let events = read_events(&mut stream, 1);
    let streamed: i64 = events[0]
        .id
        .as_deref()
        .expect("an id")
        .parse()
        .expect("a number");
    assert!(
        streamed > cursor,
        "the stream starts at the newest row, not at the beginning"
    );
}

#[test]
fn a_reconnect_resumes_above_the_last_delivered_id() {
    let home = ServeHome::new();
    let mut stream = open_stream(&home, "", None);
    save_elsewhere(&home, "the first streamed lesson");
    let first = read_events(&mut stream, 1);
    let last_id = first[0].id.clone().expect("an id");
    drop(stream);

    // Written while nothing is connected: a reconnect must still see it.
    save_elsewhere(&home, "the lesson written while disconnected");

    let mut resumed = open_stream(&home, "", Some(&last_id));
    let events = read_events(&mut resumed, 1);
    let resumed_id: i64 = events[0]
        .id
        .as_deref()
        .expect("an id")
        .parse()
        .expect("a number");
    assert!(
        resumed_id > last_id.parse::<i64>().expect("a number"),
        "Last-Event-ID resumes above the row already delivered"
    );
    assert_eq!(events[0].data["command"], "save");
}

#[test]
fn a_command_filter_streams_only_that_command() {
    let home = ServeHome::new();
    let mut stream = open_stream(&home, "?command=find", None);

    save_elsewhere(&home, "a save the filtered stream must skip");
    AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.data_dir())
        .args(["find", "pgbouncer"])
        .assert()
        .success();

    let events = read_events(&mut stream, 1);
    assert_eq!(
        events[0].data["command"], "find",
        "the save never reaches a command-filtered stream"
    );
}
