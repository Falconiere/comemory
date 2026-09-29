#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! `comemory project create --idempotency-key` through the real binary (#327):
//! a replay prints the first answer byte for byte and writes no row in any
//! project table or `activity_log`; the same key with another body is
//! `idempotency_conflict`; two processes racing one key converge on one
//! project; a create refused inside its transaction leaves no receipt, so its
//! retry runs once the obstacle is gone; and the key's length is checked.

use std::process::{Command, Output};

use comemory::store::schema_projects::PROJECT_TABLES;
use rusqlite::{Connection, OpenFlags};

#[path = "common/cli_bin.rs"]
mod cli_bin;

use cli_bin::CliHome;

/// `project create` flags for a charter named after `key_prefix`.
fn charter<'a>(key: &'a str, key_prefix: &'a str, outcome: &'a str) -> Vec<&'a str> {
    vec![
        "--json",
        "project",
        "create",
        "--name",
        key_prefix,
        "--key-prefix",
        key_prefix,
        "--outcome",
        outcome,
        "--success-criterion",
        "Tests pass",
        "--repository",
        "Falconiere/comemory",
        "--idempotency-key",
        key,
    ]
}

fn run(home: &CliHome, args: &[&str]) -> Output {
    home.bin().args(args).output().unwrap()
}

/// Run to success; its stdout bytes.
fn ok(home: &CliHome, args: &[&str]) -> Vec<u8> {
    home.run_ok(args).into_bytes()
}

/// Run to failure; its exit code and stderr.
fn refused(home: &CliHome, args: &[&str]) -> (i32, String) {
    let out = run(home, args);
    assert!(!out.status.success(), "{args:?} unexpectedly succeeded");
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn db(home: &CliHome) -> Connection {
    let path = home.data_dir().join("comemory.db");
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
}

/// `(table, rows)` for every project table, then `activity_log`.
fn counts(home: &CliHome) -> Vec<(&'static str, i64)> {
    let db = db(home);
    PROJECT_TABLES
        .iter()
        .chain(&["activity_log"])
        .map(|t| {
            let n = db
                .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                .unwrap();
            (*t, n)
        })
        .collect()
}

fn count(home: &CliHome, table: &str) -> i64 {
    counts(home)
        .into_iter()
        .find(|(t, _)| *t == table)
        .unwrap()
        .1
}

#[test]
fn a_replay_prints_the_first_answer_and_writes_nothing() {
    let home = CliHome::new();
    let first = ok(&home, &charter("k1", "SHIP", "Ship it"));
    let before = counts(&home);
    for table in [
        "projects",
        "project_command_receipts",
        "project_activity_events",
    ] {
        assert_eq!(count(&home, table), 1, "{table}");
    }

    let replayed = ok(&home, &charter("k1", "SHIP", "Ship it"));
    assert_eq!(
        String::from_utf8(replayed).unwrap(),
        String::from_utf8(first.clone()).unwrap(),
        "the replay printed another answer"
    );
    assert_eq!(counts(&home), before, "the replay wrote a row");

    // The stored response is exactly what the CLI printed.
    let stored: String = db(&home)
        .query_row("SELECT response FROM project_command_receipts", [], |r| {
            r.get(0)
        })
        .unwrap();
    let printed: serde_json::Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stored).unwrap(),
        printed
    );

    // A new key is a new command.
    ok(&home, &charter("k2", "NEXT", "Ship it"));
    assert_eq!(count(&home, "projects"), 2);
    assert_eq!(count(&home, "project_command_receipts"), 2);
}

#[test]
fn the_same_key_with_another_body_is_an_idempotency_conflict() {
    let home = CliHome::new();
    ok(&home, &charter("k1", "SHIP", "Ship it"));
    let before = counts(&home);

    let (code, stderr) = refused(&home, &charter("k1", "SHIP", "Ship something else"));
    assert_eq!(code, 75, "{stderr}");
    assert!(
        stderr.contains("This idempotency key was already used for a different command"),
        "{stderr}"
    );
    let after = counts(&home);
    for ((table, was), (_, now)) in before.iter().zip(&after) {
        let expected = if *table == "activity_log" {
            was + 1
        } else {
            *was
        };
        assert_eq!(*now, expected, "{table}");
    }
}

#[test]
fn two_processes_racing_one_key_create_one_project() {
    let bin = env!("CARGO_BIN_EXE_comemory");
    for round in 0..8 {
        let home = CliHome::new();
        // Migrate first, so the race is the create and not the schema.
        home.run_json(&["project", "list"]);
        let args = charter("race", "RACE", "Ship it");
        let spawn = || {
            Command::new(bin)
                .env("COMEMORY_DATA_DIR", home.data_dir())
                .args(&args)
                .output()
        };
        let (a, b) = std::thread::scope(|s| {
            let a = s.spawn(spawn);
            let b = s.spawn(spawn);
            (a.join().unwrap().unwrap(), b.join().unwrap().unwrap())
        });
        assert!(a.status.success(), "round {round}: {a:?}");
        assert!(b.status.success(), "round {round}: {b:?}");
        assert_eq!(a.stdout, b.stdout, "round {round}: two answers");
        for table in [
            "projects",
            "project_command_receipts",
            "project_activity_events",
        ] {
            assert_eq!(count(&home, table), 1, "round {round}: {table}");
        }
    }
}

#[test]
fn a_create_refused_in_its_transaction_leaves_no_receipt_and_its_retry_runs() {
    let home = CliHome::new();
    let first: serde_json::Value =
        serde_json::from_slice(&ok(&home, &charter("k1", "SHIP", "Ship it"))).unwrap();

    let blocked = charter("k2", "SHIP", "Another project");
    let (code, stderr) = refused(&home, &blocked);
    assert_eq!(code, 65, "{stderr}");
    assert!(stderr.contains("keyPrefix is already used"), "{stderr}");
    assert_eq!(count(&home, "project_command_receipts"), 1);
    assert_eq!(count(&home, "projects"), 1);

    // Hard deletion's shape (#320): the project goes, and its receipt with it.
    let rw = Connection::open(home.data_dir().join("comemory.db")).unwrap();
    rw.pragma_update(None, "foreign_keys", true).unwrap();
    let id = first["project"]["id"].as_str().unwrap();
    rw.execute("DELETE FROM projects WHERE id = ?1", [id])
        .unwrap();
    assert_eq!(count(&home, "project_command_receipts"), 0);

    let retried: serde_json::Value = serde_json::from_slice(&ok(&home, &blocked)).unwrap();
    assert_eq!(retried["project"]["outcome"], "Another project");
    assert_eq!(count(&home, "projects"), 1);
    assert_eq!(count(&home, "project_command_receipts"), 1);
    // k1 is free again too: its receipt went with its project.
    ok(&home, &charter("k1", "FRESH", "Ship it"));
}

#[test]
fn a_key_outside_one_to_two_hundred_characters_is_refused() {
    let home = CliHome::new();
    let long = "k".repeat(201);
    for (key, reason) in [
        ("", "idempotencyKey is too_short"),
        (long.as_str(), "idempotencyKey is too_long"),
    ] {
        let (code, stderr) = refused(&home, &charter(key, "NOPE", "Ship it"));
        assert_eq!(code, 65, "{stderr}");
        assert!(stderr.contains(reason), "{stderr}");
    }
    // Refused before the store: the database was never even opened.
    assert!(!home.data_dir().join("comemory.db").exists());
    ok(&home, &charter(&"k".repeat(200), "EDGE", "Ship it"));

    // Without a key the CLI mints one: the run works, but is not retry-safe.
    ok(
        &home,
        &[
            "--json",
            "project",
            "create",
            "--name",
            "A",
            "--key-prefix",
            "MINT",
            "--outcome",
            "o",
        ],
    );
    let keys: i64 = db(&home)
        .query_row(
            "SELECT COUNT(DISTINCT idempotency_key) FROM project_command_receipts",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(keys, 2);
}
