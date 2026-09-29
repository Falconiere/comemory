#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! `comemory project activity` through the real binary (#331): a keyset walk
//! in either order returns every event that existed before it began exactly
//! once while the test process keeps appending events to the same project
//! and spawning `project create` for others; the JSON and TTY views; and the
//! exit code and message of each refusal edge.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use comemory::store::connection;
use comemory::store::project_activity::{self, NewProjectEvent};
use serde_json::Value;

#[path = "common/cli_bin.rs"]
mod cli_bin;

use cli_bin::CliHome;

/// Events seeded before a walk starts.
const SEEDED: usize = 25;

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

/// Append one event to `project` at `at_ms` on its own connection to the
/// data directory's database, as a second writer would.
fn append(home: &CliHome, project: &str, id: &str, at_ms: i64) {
    let conn = connection::open(home.data_dir().join("comemory.db")).unwrap();
    project_activity::insert(
        &conn,
        &NewProjectEvent {
            id,
            project_id: project,
            actor_type: "project_agent",
            actor_id: "local-agent",
            event_type: "project.health_reported",
            entity_type: "project",
            entity_id: project,
            payload: r#"{"health":"on_track"}"#,
            at_ms,
        },
    )
    .unwrap();
}

/// A project with its `project.created` event and [`SEEDED`] more, in runs
/// of three sharing one millisecond; every id that exists before the walk.
fn seeded(home: &CliHome) -> (String, Vec<String>) {
    let project = create(home, "WALK");
    let mut ids: Vec<String> = activity(home, &project, &[])["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();
    for n in 0..SEEDED {
        let id = format!("{n:08x}-2222-4000-8000-000000000000");
        append(home, &project, &id, 1_000_000 + (n / 3) as i64);
        ids.push(id);
    }
    (project, ids)
}

/// `project activity <project> <args> --json`.
fn activity(home: &CliHome, project: &str, args: &[&str]) -> Value {
    let mut all = vec!["project", "activity", project];
    all.extend_from_slice(args);
    home.run_json(&all)
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

/// Page `project` to the end in `order`, 4 at a time, while the test process
/// appends events to it and charters other projects; each event id's count.
fn walk_under_writes(home: &Arc<CliHome>, project: &str, order: &str) -> HashMap<String, usize> {
    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let (home, stop, project) = (Arc::clone(home), Arc::clone(&stop), project.to_string());
        let prefix = order[..1].to_uppercase();
        std::thread::spawn(move || {
            let mut written = 0_usize;
            // Bounded, so a slow machine cannot turn this into a long loop.
            while !stop.load(Ordering::Relaxed) && written < 40 {
                let id = format!("{written:08x}-3333-4000-8000-000000000000");
                append(&home, &project, &id, comemory_now_ms());
                if written % 4 == 0 {
                    create(&home, &format!("{prefix}{written}"));
                }
                written += 1;
            }
            written
        })
    };
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut args = vec!["--order", order, "--limit", "4"];
        if let Some(c) = &cursor {
            args.extend(["--cursor", c.as_str()]);
        }
        let page = activity(home, project, &args);
        let events = page["events"].as_array().unwrap();
        assert!(events.len() <= 4, "{page}");
        for e in events {
            assert_eq!(e["projectId"], project, "another project's event: {e}");
            *seen
                .entry(e["id"].as_str().unwrap().to_string())
                .or_default() += 1;
        }
        match page["nextCursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }
    stop.store(true, Ordering::Relaxed);
    assert!(
        writer.join().unwrap() > 0,
        "the concurrent writer never ran"
    );
    seen
}

/// The wall clock in epoch milliseconds, the stamp a live mutation writes.
fn comemory_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

#[test]
fn a_walk_returns_every_earlier_event_once_while_another_process_writes() {
    for order in ["desc", "asc"] {
        let home = Arc::new(CliHome::new());
        let (project, ids) = seeded(&home);
        assert_eq!(ids.len(), SEEDED + 1);
        let seen = walk_under_writes(&home, &project, order);
        for id in &ids {
            assert_eq!(
                seen.get(id),
                Some(&1),
                "{order}: event {id} seen {:?} times",
                seen.get(id)
            );
        }
        assert!(
            seen.values().all(|&n| n == 1),
            "{order}: an event appeared twice"
        );
    }
}

#[test]
fn json_and_tty_views_page_one_project_and_refusals_exit_by_edge() {
    let home = CliHome::new();
    let project = create(&home, "SHOW");
    let other = create(&home, "OTHER");
    let page = activity(&home, &project, &[]);
    assert_eq!(page["nextCursor"], Value::Null);
    let events = page["events"].as_array().unwrap();
    assert_eq!(events.len(), 1, "{page}");
    assert_eq!(events[0]["eventType"], "project.created");
    assert_eq!(events[0]["entityId"], project.as_str());
    assert_eq!(events[0]["actorPrincipalId"], "local-operator");
    assert_ne!(events[0]["projectId"], other.as_str());

    append(
        &home,
        &project,
        "00000001-2222-4000-8000-000000000000",
        comemory_now_ms() + 60_000,
    );
    let tty = home.run_ok(&["project", "activity", &project, "--limit", "1"]);
    let lines: Vec<&str> = tty.lines().collect();
    assert_eq!(lines.len(), 2, "{tty}");
    assert!(
        lines[0].contains("project.health_reported")
            && lines[0].contains(&format!("project {project}  project_agent:local-agent")),
        "{tty}"
    );
    assert!(lines[1].starts_with("next page: --cursor "), "{tty}");
    let cursor = lines[1].trim_start_matches("next page: --cursor ");
    let last = home.run_ok(&[
        "project", "activity", &project, "--limit", "1", "--cursor", cursor,
    ]);
    assert!(
        last.contains("project.created") && last.contains("user:local-operator"),
        "{last}"
    );
    assert!(!last.contains("next page"), "{last}");
    let asc = home.run_ok(&["project", "activity", &project, "--order", "asc"]);
    let asc: Vec<&str> = asc.lines().collect();
    assert!(asc[0].contains("project.created") && asc[1].contains("project.health_reported"));

    let edges: [(&[&str], i32, &str); 6] = [
        (&["not-a-uuid"], 64, "projectId is invalid"),
        (
            &["00000000-0000-4000-8000-000000000000"],
            64,
            "Project not found",
        ),
        (&[&project, "--cursor", "abc"], 64, "cursor is invalid"),
        (&[&project, "--order", "sideways"], 64, "order is invalid"),
        (
            &[&project, "--limit", "201"],
            65,
            "limit is too_large (limit 200)",
        ),
        (
            &[&project, "--limit", "0"],
            65,
            "limit is too_small (limit 1)",
        ),
    ];
    for (args, code, message) in edges {
        let mut all = vec!["project", "activity"];
        all.extend_from_slice(args);
        let (got, stderr) = refused(&home, &all);
        assert_eq!(got, code, "{args:?}: {stderr}");
        assert!(stderr.contains(message), "{args:?}: {stderr}");
    }
    for limit in ["1", "200"] {
        activity(&home, &project, &["--limit", limit]);
    }
}
