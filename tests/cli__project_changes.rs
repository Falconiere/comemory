#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! The body-free project change feed (#324) through real processes: a spawned
//! `comemory serve` and the real CLI on one data directory. One frame per
//! committed mutation and none for a refused or replayed one; frames carry
//! ids only, asserted on the emitted bytes; a reader that drops its socket
//! mid-response or outlives a server restart resumes from its cursor with no
//! gap and no duplicate; out-of-window cursors are refused over HTTP and the
//! CLI; and `comemory rebuild` keeps every `seq`.

#[path = "common/replica_support.rs"]
mod replica_support;

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;

use assert_cmd::cargo::cargo_bin;
use replica_support::Engine;
use serde_json::Value;

/// Strings only the charter carries: none may reach a frame.
const CHARTER_TEXT: &[&str] = &[
    "Zephyr charter name",
    "Quokka outcome sentence",
    "Narwhal success criterion",
    "Axolotl constraint",
    "Pangolin non-goal",
    "falconiere/okapi-repository",
];

/// The keys every frame carries, and no others.
const FRAME_KEYS: &[&str] = &["entity", "event_id", "op", "project_id", "seq"];

/// `comemory <args>` on `data_dir`: `(exit code, stdout, stderr)`.
fn run(data_dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", data_dir)
        .args(args)
        .output()
        .expect("run comemory");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// `project create` for `key` through the CLI; the new project's id.
fn create(data_dir: &Path, key: &str) -> String {
    let (code, stdout, stderr) = run(
        data_dir,
        &[
            "--json",
            "project",
            "create",
            "--name",
            &format!("Project {key}"),
            "--key-prefix",
            key,
            "--outcome",
            "Ship it",
        ],
    );
    assert_eq!(code, 0, "{stderr}");
    let created: Value = serde_json::from_str(&stdout).unwrap();
    created["project"]["id"].as_str().unwrap().to_string()
}

/// `GET <path>` with the engine's token: `(status, raw body)`.
fn get_raw(engine: &Engine, path: &str) -> (u16, String) {
    let response = reqwest::blocking::Client::new()
        .get(format!("{}{path}", engine.base))
        .bearer_auth(&engine.token)
        .send()
        .expect("GET");
    (response.status().as_u16(), response.text().expect("body"))
}

/// One HTTP page of frames after `after`.
fn page(engine: &Engine, after: i64, limit: i64) -> Vec<Value> {
    let (status, body) = get_raw(
        engine,
        &format!("/api/v1/projects/changes?after={after}&limit={limit}"),
    );
    assert_eq!(status, 200, "{body}");
    let envelope: Value = serde_json::from_str(&body).unwrap();
    envelope["data"]
        .as_array()
        .expect("data is the frame array")
        .clone()
}

/// Each frame's `seq`.
fn seqs(frames: &[Value]) -> Vec<i64> {
    frames.iter().map(|f| f["seq"].as_i64().unwrap()).collect()
}

/// `(seq, project_id, event_id, op)` of every stored feed row.
fn feed_rows(data_dir: &Path) -> Vec<(i64, String, String, String)> {
    let conn = rusqlite::Connection::open(data_dir.join("comemory.db")).unwrap();
    let mut stmt = conn
        .prepare("SELECT seq, project_id, event_id, op FROM project_changes ORDER BY seq")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Assert `frames_json` is an array of frames with exactly [`FRAME_KEYS`] and
/// that the raw bytes carry none of [`CHARTER_TEXT`].
fn assert_body_free(raw: &str, frames: &[Value]) {
    assert!(!frames.is_empty(), "{raw}");
    for frame in frames {
        let keys: Vec<&str> = frame
            .as_object()
            .expect("frame is an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, FRAME_KEYS, "{frame}");
        assert_eq!(frame["entity"], "project");
    }
    for text in CHARTER_TEXT {
        assert!(!raw.contains(text), "{text:?} leaked into {raw}");
    }
    assert!(
        !raw.contains("project.created"),
        "an event type leaked: {raw}"
    );
}

#[test]
fn each_committed_mutation_emits_one_body_free_frame() {
    let engine = Engine::spawn(&[]);
    let dir = engine.data_dir();
    let id = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2a";
    let charter = [
        "--json",
        "project",
        "create",
        "--id",
        id,
        "--name",
        CHARTER_TEXT[0],
        "--key-prefix",
        "ZEPH",
        "--outcome",
        CHARTER_TEXT[1],
        "--success-criterion",
        CHARTER_TEXT[2],
        "--constraint",
        CHARTER_TEXT[3],
        "--non-goal",
        CHARTER_TEXT[4],
        "--repository",
        CHARTER_TEXT[5],
    ];
    let (code, _, stderr) = run(&dir, &charter);
    assert_eq!(code, 0, "{stderr}");
    let after_create = feed_rows(&dir);

    // A replay of the same command, and two refusals: no frame.
    let (code, _, _) = run(&dir, &charter);
    assert_eq!(code, 65, "a replay with the same id is refused");
    let (code, _, _) = run(
        &dir,
        &[
            "project",
            "create",
            "--name",
            "Other",
            "--key-prefix",
            "ZEPH",
            "--outcome",
            "o",
        ],
    );
    assert_eq!(code, 65, "a taken key prefix is refused");
    let (code, _, _) = run(
        &dir,
        &[
            "project",
            "create",
            "--name",
            "",
            "--key-prefix",
            "EMPTY",
            "--outcome",
            "o",
        ],
    );
    assert_eq!(code, 65, "an empty name is refused");
    assert_eq!(
        feed_rows(&dir),
        after_create,
        "a failing command wrote a frame"
    );

    let conn = rusqlite::Connection::open(dir.join("comemory.db")).unwrap();
    let event: String = conn
        .query_row("SELECT id FROM project_activity_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        after_create,
        [(1, id.to_string(), event.clone(), "changed".to_string())]
    );

    let (status, raw) = get_raw(&engine, "/api/v1/projects/changes");
    assert_eq!(status, 200, "{raw}");
    let envelope: Value = serde_json::from_str(&raw).unwrap();
    let frames = envelope["data"].as_array().unwrap();
    assert_body_free(&raw, frames);
    assert_eq!(frames[0]["event_id"], event.as_str());
    assert_eq!(frames[0]["project_id"], id);

    let (code, stdout, stderr) = run(&dir, &["--json", "project", "changes"]);
    assert_eq!(code, 0, "{stderr}");
    let cli_frames: Value = serde_json::from_str(&stdout).unwrap();
    assert_body_free(&stdout, cli_frames.as_array().unwrap());
    assert_eq!(
        &cli_frames, &envelope["data"],
        "CLI and HTTP print one frame shape"
    );

    // `changes` is a static segment, never read as a project id.
    let (status, raw) = get_raw(&engine, &format!("/api/v1/projects/{id}"));
    assert_eq!(status, 200, "{raw}");
}

#[test]
fn a_reader_resumes_after_a_mid_stream_drop_and_a_server_restart() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join(".comemory");
    let mut ids: Vec<String> = ["AA", "BB", "CC"].iter().map(|k| create(&dir, k)).collect();
    let mut seen: Vec<Value> = Vec::new();
    let mut cursor = 0;

    let engine = Engine::spawn_at(&dir, &[]);
    let first = page(&engine, cursor, 1);
    cursor = first[0]["seq"].as_i64().unwrap();
    seen.extend(first);

    // Mid-stream drop: the reader sends its next read, takes a few bytes of
    // the response and closes the socket. It never saw a whole page, so its
    // cursor stays where it was.
    let port = engine.base.rsplit(':').next().unwrap();
    let mut socket = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    write!(
        socket,
        "GET /api/v1/projects/changes?after={cursor}&limit=1000 HTTP/1.1\r\n\
         Host: 127.0.0.1\r\nAuthorization: Bearer {}\r\n\r\n",
        engine.token
    )
    .unwrap();
    let mut partial = [0_u8; 16];
    socket.read_exact(&mut partial).unwrap();
    assert!(partial.starts_with(b"HTTP/1.1 200"), "{partial:?}");
    drop(socket);

    ids.push(create(&dir, "DD"));
    let resumed = page(&engine, cursor, 2);
    cursor = resumed.last().unwrap()["seq"].as_i64().unwrap();
    seen.extend(resumed);

    // Server restart between pages, with mutations committed while it is down.
    drop(engine);
    ids.extend(["EE", "FF"].iter().map(|k| create(&dir, k)));
    let engine = Engine::spawn_at(&dir, &[]);
    loop {
        let next = page(&engine, cursor, 1);
        let Some(last) = next.last() else { break };
        cursor = last["seq"].as_i64().unwrap();
        seen.extend(next);
    }

    let stored: Vec<i64> = feed_rows(&dir).iter().map(|r| r.0).collect();
    assert_eq!(
        seqs(&seen),
        stored,
        "every frame exactly once, in seq order"
    );
    assert_eq!(stored, [1, 2, 3, 4, 5, 6]);
    let announced: BTreeSet<&str> = seen
        .iter()
        .map(|f| f["project_id"].as_str().unwrap())
        .collect();
    let created: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    assert_eq!(announced, created);

    let (status, raw) = get_raw(&engine, "/api/v1/projects?limit=100");
    assert_eq!(status, 200, "{raw}");
    let listed: Value = serde_json::from_str(&raw).unwrap();
    let listed: BTreeSet<&str> = listed["data"]["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        listed, created,
        "a read afterwards returns the committed state"
    );
}

#[test]
fn cursors_outside_the_feed_are_refused_over_http_and_the_cli() {
    let engine = Engine::spawn(&[]);
    let dir = engine.data_dir();
    for key in ["AA", "BB", "CC"] {
        create(&dir, key);
    }
    let refusal = |path: &str| {
        let (status, raw) = get_raw(&engine, path);
        let body: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(body["error"]["code"], "invalid_request", "{raw}");
        (status, body["error"]["details"].clone())
    };
    let cases = [
        (
            "after=4",
            422,
            r#"{"field":"after","reason":"cursor_ahead"}"#,
        ),
        ("after=-1", 400, r#"{"field":"after","reason":"invalid"}"#),
        ("after=x", 400, r#"{"field":"after","reason":"invalid"}"#),
        (
            "limit=0",
            422,
            r#"{"field":"limit","reason":"too_small","limit":1}"#,
        ),
        (
            "limit=1001",
            422,
            r#"{"field":"limit","reason":"too_large","limit":1000}"#,
        ),
        ("since=0", 400, r#"{"field":"since","reason":"invalid"}"#),
    ];
    for (query, status, details) in cases {
        let expected: Value = serde_json::from_str(details).unwrap();
        assert_eq!(
            refusal(&format!("/api/v1/projects/changes?{query}")),
            (status, expected),
            "{query}"
        );
    }
    assert_eq!(
        seqs(&page(&engine, 3, 10)),
        Vec::<i64>::new(),
        "the head is valid"
    );

    let cli = |args: &[&str]| {
        let mut all = vec!["project", "changes"];
        all.extend(args);
        run(&dir, &all)
    };
    let (code, _, stderr) = cli(&["--after", "4"]);
    assert_eq!(code, 65, "{stderr}");
    assert!(stderr.contains("after is cursor_ahead"), "{stderr}");
    let (code, _, stderr) = cli(&["--limit", "1001"]);
    assert_eq!(code, 65, "{stderr}");
    assert!(
        stderr.contains("limit is too_large (limit 1000)"),
        "{stderr}"
    );
    let (code, _, stderr) = cli(&["--after", "x"]);
    assert_eq!(code, 2, "clap refuses a non-integer: {stderr}");

    // A pruned window: readers at 0 and 1 would skip rows they never saw.
    let conn = rusqlite::Connection::open(dir.join("comemory.db")).unwrap();
    conn.execute("DELETE FROM project_changes WHERE seq <= 2", [])
        .unwrap();
    let expired: Value =
        serde_json::from_str(r#"{"field":"after","reason":"cursor_expired"}"#).unwrap();
    assert_eq!(refusal("/api/v1/projects/changes?after=0"), (422, expired));
    let (code, _, stderr) = cli(&["--after", "1"]);
    assert_eq!(code, 65, "{stderr}");
    assert!(stderr.contains("after is cursor_expired"), "{stderr}");
    assert_eq!(seqs(&page(&engine, 2, 10)), [3]);
}

#[test]
fn a_rebuild_keeps_every_seq_so_a_cursor_stays_valid() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join(".comemory");
    for key in ["AA", "BB"] {
        create(&dir, key);
    }
    let before = feed_rows(&dir);
    let (code, _, stderr) = run(&dir, &["rebuild"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(feed_rows(&dir), before, "rows and seqs survive the rebuild");

    create(&dir, "CC");
    let (code, stdout, stderr) = run(&dir, &["--json", "project", "changes", "--after", "2"]);
    assert_eq!(code, 0, "{stderr}");
    let frames: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(seqs(frames.as_array().unwrap()), [3], "the cursor resumes");

    let (code, stdout, _) = run(&dir, &["project", "changes", "--after", "2"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("next: --after 3"), "{stdout}");
}
