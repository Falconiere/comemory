#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `comemory project archive|restore|pause|resume` through the real binary
//! on a fresh data directory (#328): the lifecycle walk with its version
//! bumps and one activity event per success, each refusal's exit code and
//! message, the archived project leaving and rejoining the default list, and
//! a named key replaying without a second write.

use serde_json::Value;

#[path = "common/cli_bin.rs"]
mod cli_bin;

use cli_bin::CliHome;

/// Run `comemory <args>` expecting failure; `(exit code, stderr)`.
fn refused(home: &CliHome, args: &[&str]) -> (i32, String) {
    let out = home.bin().args(args).output().unwrap();
    assert!(!out.status.success(), "{args:?} unexpectedly succeeded");
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// `project <verb> <id> --expected-version <version>` plus `extra`, as JSON.
fn lifecycle(home: &CliHome, verb: &str, id: &str, version: i64, extra: &[&str]) -> Value {
    let version = version.to_string();
    let mut args = vec!["project", verb, id, "--expected-version", &version];
    args.extend_from_slice(extra);
    home.run_json(&args)["project"].clone()
}

/// The project's activity event types, oldest first.
fn events(home: &CliHome, id: &str) -> Vec<String> {
    let page = home.run_json(&["project", "activity", id, "--order", "asc"]);
    page["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["eventType"].as_str().unwrap().to_string())
        .collect()
}

/// The ids `project list` returns, with or without archived projects.
fn listed(home: &CliHome, include_archived: bool) -> Vec<String> {
    let mut args = vec!["project", "list"];
    if include_archived {
        args.push("--include-archived");
    }
    home.run_json(&args)["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_project_walks_its_lifecycle_offline() {
    let home = CliHome::new();
    let created = home.run_json(&[
        "project",
        "create",
        "--name",
        "Lifecycle",
        "--key-prefix",
        "LIFE",
        "--outcome",
        "Ship it",
    ]);
    let id = created["project"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["project"]["version"], 1);

    // A draft project cannot be paused, and a pause needs a real reason.
    let pause = ["project", "pause", &id, "--expected-version", "1"];
    let (code, stderr) = refused(&home, &[&pause[..], &["--reason", "wait"]].concat());
    assert_eq!(code, 75, "{stderr}");
    assert!(
        stderr.contains("Only an active project can be paused"),
        "{stderr}"
    );
    let (code, stderr) = refused(&home, &pause);
    assert_eq!(code, 64, "{stderr}");
    assert!(stderr.contains("reason is required"), "{stderr}");
    let (code, stderr) = refused(&home, &[&pause[..], &["--reason", "   "]].concat());
    assert_eq!(code, 65, "{stderr}");
    assert!(stderr.contains("reason is blank"), "{stderr}");

    // No verb reaches `active` yet (plan approval will): seed it.
    let db = rusqlite::Connection::open(home.data_dir().join("comemory.db")).unwrap();
    db.execute("UPDATE projects SET status = 'active' WHERE id = ?1", [&id])
        .unwrap();

    let paused = lifecycle(&home, "pause", &id, 1, &["--reason", "Design review"]);
    assert_eq!(
        (paused["status"].as_str(), paused["version"].as_i64()),
        (Some("paused"), Some(2))
    );

    let (code, stderr) = refused(
        &home,
        &["project", "resume", &id, "--expected-version", "1"],
    );
    assert_eq!(code, 75, "{stderr}");
    assert!(
        stderr.contains("The project has changed since it was last read"),
        "{stderr}"
    );

    // The TTY view names the version the next command needs.
    let tty = home.run_ok(&["project", "resume", &id, "--expected-version", "2"]);
    assert!(tty.contains("status        active (unknown)"), "{tty}");
    assert!(tty.contains("version       3"), "{tty}");
    assert!(tty.contains("archived      -"), "{tty}");

    let archived = lifecycle(&home, "archive", &id, 3, &[]);
    assert_eq!(archived["version"], 4);
    assert!(archived["archivedAt"].is_string(), "{archived}");
    assert!(listed(&home, false).is_empty());
    assert_eq!(listed(&home, true), [id.clone()]);

    let (code, stderr) = refused(
        &home,
        &[
            "project",
            "pause",
            &id,
            "--expected-version",
            "4",
            "--reason",
            "x",
        ],
    );
    assert_eq!(code, 75, "{stderr}");
    assert!(stderr.contains("This project is archived"), "{stderr}");

    let restored = lifecycle(&home, "restore", &id, 4, &["--reason", "Back on"]);
    assert_eq!(
        (restored["version"].as_i64(), &restored["archivedAt"]),
        (Some(5), &Value::Null)
    );
    assert_eq!(listed(&home, false), [id.clone()]);

    assert_eq!(
        events(&home, &id),
        [
            "project.created",
            "project.paused",
            "project.resumed",
            "project.archived",
            "project.restored",
        ]
    );
}

#[test]
fn a_named_key_replays_and_an_unknown_project_is_not_found() {
    let home = CliHome::new();
    let created = home.run_json(&[
        "project",
        "create",
        "--name",
        "Replay",
        "--key-prefix",
        "REPLAY",
        "--outcome",
        "Ship it",
    ]);
    let id = created["project"]["id"].as_str().unwrap().to_string();
    let keyed = ["--idempotency-key", "archive-once"];
    let first = lifecycle(&home, "archive", &id, 1, &keyed);
    let again = lifecycle(&home, "archive", &id, 1, &keyed);
    assert_eq!(again, first);
    assert_eq!(first["version"], 2);
    assert_eq!(events(&home, &id).len(), 2, "a replay wrote an event");

    let (code, stderr) = refused(
        &home,
        &[
            "project",
            "restore",
            &id,
            "--expected-version",
            "2",
            "--idempotency-key",
            "archive-once",
        ],
    );
    assert_eq!(code, 75, "{stderr}");
    assert!(
        stderr.contains("This idempotency key was already used for a different command"),
        "{stderr}"
    );

    let unknown = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f";
    let (code, stderr) = refused(
        &home,
        &["project", "archive", unknown, "--expected-version", "1"],
    );
    assert_eq!(code, 64, "{stderr}");
    assert!(stderr.contains("Project not found"), "{stderr}");
}
