#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! `comemory project` through the real binary on a fresh data directory with
//! no credential (#326): create → show → list offline, the exit code and
//! message of each refusal edge, a refused create leaving nothing behind,
//! and a keyset walk that returns every pre-existing project exactly once
//! while another process keeps creating projects.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

#[path = "common/cli_bin.rs"]
mod cli_bin;

use cli_bin::CliHome;

/// `project create` with a generated name for `key`, returning the new id.
fn create(home: &CliHome, key: &str) -> String {
    let created = home.run_json(&[
        "project",
        "create",
        "--name",
        &format!("Project {key}"),
        "--key-prefix",
        key,
        "--outcome",
        "Ship it",
    ]);
    created["project"]["id"].as_str().unwrap().to_string()
}

/// Run `comemory <args>` expecting failure; `(exit code, stderr)`.
fn refused(home: &CliHome, args: &[&str]) -> (i32, String) {
    let out = home.bin().args(args).output().unwrap();
    assert!(!out.status.success(), "{args:?} unexpectedly succeeded");
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn a_project_is_created_shown_and_listed_offline() {
    let home = CliHome::new();
    assert!(
        !home.data_dir().exists(),
        "the data directory starts absent"
    );
    let created = home.run_json(&[
        "project",
        "create",
        "--name",
        "Ship offline projects",
        "--key-prefix",
        "SHIP",
        "--outcome",
        "Projects work without a cloud account",
        "--success-criterion",
        "A project is created offline",
        "--constraint",
        "No network",
        "--non-goal",
        "A mesh",
        "--repository",
        "Falconiere/comemory",
        "--target-date",
        "2026-10-01",
    ]);
    let project = &created["project"];
    let id = project["id"].as_str().unwrap();
    assert_eq!(project["status"], "draft");
    assert_eq!(project["slug"], "ship-offline-projects");
    assert_eq!(project["leadUserId"], "local-operator");
    assert_eq!(
        project["repositories"],
        serde_json::json!(["falconiere/comemory"])
    );
    assert_eq!(
        project["criteria"][0]["description"],
        "A project is created offline"
    );
    assert_eq!(project["targetDate"], "2026-10-01T00:00:00.000Z");

    let shown = home.run_json(&["project", "show", id]);
    assert_eq!(&shown["project"], project);
    let listed = home.run_json(&["project", "list"]);
    assert_eq!(listed["projects"], serde_json::json!([project]));
    assert_eq!(listed["nextCursor"], Value::Null);
    let tty = home.run_ok(&["project", "list"]);
    assert!(tty.contains("SHIP") && tty.contains(id), "{tty}");
}

#[test]
fn refusals_exit_by_edge_and_leave_no_row() {
    let home = CliHome::new();
    create(&home, "SHIP");

    let (code, stderr) = refused(
        &home,
        &[
            "project",
            "create",
            "--name",
            "Dup",
            "--key-prefix",
            "SHIP",
            "--outcome",
            "o",
        ],
    );
    assert_eq!(code, 65, "{stderr}");
    assert!(
        stderr.contains("keyPrefix is already used in this workspace"),
        "{stderr}"
    );

    let long = "n".repeat(121);
    let (code, stderr) = refused(
        &home,
        &[
            "project",
            "create",
            "--name",
            &long,
            "--key-prefix",
            "LONG",
            "--outcome",
            "o",
        ],
    );
    assert_eq!(code, 65, "{stderr}");
    assert!(stderr.contains("name is too_long (limit 120)"), "{stderr}");

    let (code, stderr) = refused(&home, &["project", "list", "--limit", "101"]);
    assert_eq!(code, 65, "{stderr}");
    assert!(
        stderr.contains("limit is too_large (limit 100)"),
        "{stderr}"
    );

    let (code, stderr) = refused(&home, &["project", "list", "--cursor", "abc"]);
    assert_eq!(code, 64, "{stderr}");
    assert!(stderr.contains("cursor is invalid"), "{stderr}");

    let (code, _) = refused(&home, &["project", "show", "not-a-uuid"]);
    assert_eq!(code, 64);
    let (code, stderr) = refused(
        &home,
        &["project", "show", "00000000-0000-4000-8000-000000000000"],
    );
    assert_eq!(code, 64, "{stderr}");
    assert!(stderr.contains("Project not found"), "{stderr}");

    let listed = home.run_json(&["project", "list"]);
    assert_eq!(
        listed["projects"].as_array().unwrap().len(),
        1,
        "a refused create left a row"
    );
}

#[test]
fn a_walk_returns_every_earlier_project_once_while_another_process_inserts() {
    let home = Arc::new(CliHome::new());
    let seeded: Vec<String> = (0..30).map(|n| create(&home, &format!("S{n}"))).collect();

    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let (home, stop) = (Arc::clone(&home), Arc::clone(&stop));
        std::thread::spawn(move || {
            let mut written = 0;
            // Bounded, so a slow machine cannot turn this into a long loop.
            while !stop.load(Ordering::Relaxed) && written < 40 {
                create(&home, &format!("W{written}"));
                written += 1;
            }
            written
        })
    };

    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut args = vec!["project", "list", "--limit", "3"];
        if let Some(c) = &cursor {
            args.extend(["--cursor", c.as_str()]);
        }
        let page = home.run_json(&args);
        for p in page["projects"].as_array().unwrap() {
            *seen
                .entry(p["id"].as_str().unwrap().to_string())
                .or_default() += 1;
        }
        match page["nextCursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }
    stop.store(true, Ordering::Relaxed);
    let written = writer.join().unwrap();
    assert!(written > 0, "the concurrent writer never ran");
    for id in &seeded {
        assert_eq!(
            seen.get(id),
            Some(&1),
            "seeded project {id} seen {:?} times",
            seen.get(id)
        );
    }
    assert!(seen.values().all(|&n| n == 1), "a project appeared twice");
}

#[test]
fn every_flag_reaches_the_core() {
    let home = CliHome::new();
    let id = "0F8C2D7E-3B1A-4C5D-9E6F-7A8B9C0D1E2F";
    let created = home.run_json(&[
        "project",
        "create",
        "--id",
        id,
        "--lead",
        "lead-7",
        "--name",
        "Flags",
        "--key-prefix",
        "FLAG",
        "--outcome",
        "o",
        "--target-date",
        "2026-10-01T02:00:00+02:00",
    ]);
    assert_eq!(created["project"]["id"], id.to_lowercase());
    assert_eq!(created["project"]["leadUserId"], "lead-7");
    assert_eq!(created["project"]["targetDate"], "2026-10-01T00:00:00.000Z");
    let listed = home.run_json(&[
        "project",
        "list",
        "--status",
        "draft",
        "--health",
        "unknown",
        "--include-archived",
    ]);
    assert_eq!(listed["projects"][0]["id"], id.to_lowercase());
    let none = home.run_json(&["project", "list", "--status", "active"]);
    assert_eq!(none["projects"], serde_json::json!([]));
    let (code, stderr) = refused(&home, &["project", "list", "--health", "great"]);
    assert_eq!(code, 64, "{stderr}");
    assert!(stderr.contains("health is invalid"), "{stderr}");
}
