#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! `comemory project evidence add|list` through the real binary (#346):
//! evidence attaches at project and item level; every list filter, alone and
//! combined, returns the right rows and a `--limit` walk ends on a `null`
//! cursor; a stored unknown trust reads as `invalid`; a retried key prints
//! the first answer; and every refusal exits by its edge having stored
//! nothing.

use serde_json::{Value, json};

#[path = "common/cli_bin.rs"]
mod cli_bin;

use cli_bin::CliHome;

/// The project `plan_seed.sql` writes its plan into.
const PROJECT: &str = "11111111-1111-4111-8111-111111111111";
const ITEM: &str = "b0000000-0000-4000-8000-000000000002";
const CRITERION: &str = "c0000000-0000-4000-8000-000000000001";
const SHA: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// A data directory with [`PROJECT`] (repository `falconiere/comemory`) and
/// its committed plan seeded straight into the store.
fn home() -> CliHome {
    let home = CliHome::new();
    home.run_json(&[
        "project",
        "create",
        "--id",
        PROJECT,
        "--name",
        "Evidence",
        "--key-prefix",
        "EVID",
        "--outcome",
        "Evidence lands",
        "--repository",
        "falconiere/comemory",
    ]);
    sql(&home, include_str!("fixtures/projects/plan_seed.sql"));
    home
}

fn sql(home: &CliHome, batch: &str) {
    let conn = rusqlite::Connection::open(home.data_dir().join("comemory.db")).unwrap();
    conn.execute_batch(batch).unwrap();
}

/// `[evidence rows, recorded events, evidence receipts]`.
fn counts(home: &CliHome) -> [i64; 3] {
    let conn = rusqlite::Connection::open(home.data_dir().join("comemory.db")).unwrap();
    conn.query_row(
        "SELECT (SELECT count(*) FROM project_evidence),
                (SELECT count(*) FROM project_activity_events
                  WHERE event_type = 'project.evidence.recorded'),
                (SELECT count(*) FROM project_command_receipts
                  WHERE command_type = 'project.evidence.create')",
        [],
        |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?]),
    )
    .unwrap()
}

/// `project evidence add PROJECT <args> --json`, the evidence view.
fn add(home: &CliHome, args: &[&str]) -> Value {
    let mut all = vec!["project", "evidence", "add", PROJECT];
    all.extend_from_slice(args);
    home.run_json(&all)["evidence"].clone()
}

/// `project evidence list PROJECT <args> --json`.
fn list(home: &CliHome, args: &[&str]) -> Value {
    let mut all = vec!["project", "evidence", "list", PROJECT];
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

#[test]
fn evidence_attaches_at_both_levels_and_every_filter_pages_the_right_rows() {
    let home = home();
    let git = ["--repo", "falconiere/comemory", "--commit-sha", SHA];
    let mut ids = Vec::new();
    for args in [
        vec![
            "--kind",
            "external_url",
            "--source",
            "ci",
            "--url",
            "https://ci.example/7",
        ],
        [
            vec!["--kind", "commit", "--source", "git", "--work-item", ITEM],
            git.to_vec(),
        ]
        .concat(),
        vec!["--kind", "test_run", "--source", "ci", "--work-item", ITEM],
        [
            vec![
                "--kind",
                "commit",
                "--source",
                "git",
                "--criterion",
                CRITERION,
            ],
            git.to_vec(),
        ]
        .concat(),
        vec![
            "--kind",
            "memory",
            "--source",
            "comemory",
            "--external-id",
            "ab12cd34",
        ],
    ] {
        let shown = add(&home, &args);
        ids.push(shown["id"].as_str().unwrap().to_string());
        // One millisecond apart at least, so newest-first order is known.
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    let project_level = list(&home, &["--kind", "external_url"]);
    assert_eq!(project_level["evidence"][0]["workItemId"], Value::Null);
    assert_eq!(project_level["evidence"][0]["trust"], "self_reported");
    let item_level = list(&home, &["--work-item", ITEM]);
    assert_eq!(item_level["evidence"][0]["workItemId"], ITEM);
    assert_eq!(counts(&home), [5, 5, 5]);

    let picked = |args: &[&str]| -> Vec<usize> {
        list(&home, args)["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| ids.iter().position(|id| e["id"] == id.as_str()).unwrap())
            .collect()
    };
    let cases: [(&[&str], Vec<usize>); 8] = [
        (&[], vec![4, 3, 2, 1, 0]),
        (&["--kind", "commit"], vec![3, 1]),
        (&["--trust", "pending"], vec![4, 3, 1]),
        (&["--work-item", ITEM], vec![2, 1]),
        (&["--kind", "commit", "--trust", "pending"], vec![3, 1]),
        (&["--kind", "commit", "--work-item", ITEM], vec![1]),
        (&["--trust", "self_reported", "--work-item", ITEM], vec![2]),
        (
            &[
                "--kind",
                "test_run",
                "--trust",
                "self_reported",
                "--work-item",
                ITEM,
            ],
            vec![2],
        ),
    ];
    for (args, expected) in cases {
        assert_eq!(picked(args), expected, "{args:?}");
    }

    // A stored trust this build does not know reads as the least trusted.
    sql(
        &home,
        &format!(
            "UPDATE project_evidence SET trust = 'bogus' WHERE id = '{}'",
            ids[0]
        ),
    );
    let invalid = list(&home, &["--trust", "invalid"]);
    assert_eq!(invalid["evidence"][0]["id"], ids[0].as_str());
    assert_eq!(invalid["evidence"][0]["trust"], "invalid");

    // A --limit walk visits each row once and ends on a null cursor.
    let mut walked = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut args = vec!["--limit", "2"];
        if let Some(c) = &cursor {
            args.extend(["--cursor", c.as_str()]);
        }
        let page = list(&home, &args);
        walked.extend(
            page["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["id"].clone()),
        );
        match page["nextCursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }
    let expected: Vec<Value> = ids.iter().rev().map(|id| json!(id)).collect();
    assert_eq!(walked, expected);

    let tty = home.run_ok(&["project", "evidence", "list", PROJECT, "--limit", "1"]);
    assert!(tty.contains("pending") && tty.contains("memory"), "{tty}");
    let row = tty.lines().next().unwrap();
    assert!(
        row.contains(&format!("{}  project  comemory", ids[4])),
        "{tty}"
    );
    assert!(
        tty.lines()
            .last()
            .unwrap()
            .starts_with("next page: --cursor "),
        "{tty}"
    );
}

#[test]
fn a_retried_key_prints_the_first_answer_and_a_reused_one_conflicts() {
    let home = home();
    let args = [
        "--kind",
        "decision",
        "--source",
        "adr",
        "--external-id",
        "adr-7",
        "--idempotency-key",
        "adr-7-once",
    ];
    let first = add(&home, &args);
    assert_eq!(first["trust"], "pending");
    assert_eq!(add(&home, &args), first);
    assert_eq!(counts(&home), [1, 1, 1]);
    let mut changed = vec!["project", "evidence", "add", PROJECT];
    changed.extend_from_slice(&args);
    assert_eq!(changed[7], "adr");
    changed[7] = "adr-log";
    let (code, stderr) = refused(&home, &changed);
    assert_eq!(code, 75, "{stderr}");
    assert!(
        stderr.contains("already used for a different command"),
        "{stderr}"
    );
    assert_eq!(counts(&home), [1, 1, 1]);
}

#[test]
fn every_refusal_exits_by_its_edge_and_stores_nothing() {
    let home = home();
    let long_sha = "a".repeat(257);
    let commit = |extra: &[&str]| -> Vec<String> {
        let mut all = vec![
            "project", "evidence", "add", PROJECT, "--kind", "commit", "--source", "git",
        ];
        all.extend_from_slice(extra);
        all.into_iter().map(String::from).collect()
    };
    let owned = |args: &[&str]| -> Vec<String> { args.iter().map(|a| (*a).to_string()).collect() };
    let edges: Vec<(Vec<String>, i32, &str)> = vec![
        (
            commit(&["--repo", "not-a-repo", "--commit-sha", SHA]),
            65,
            "repo is invalid_format",
        ),
        (
            commit(&["--repo", "falconiere/comemory", "--commit-sha", "xyz"]),
            65,
            "commitSha is invalid_format",
        ),
        (
            commit(&["--repo", "falconiere/comemory", "--commit-sha", &long_sha]),
            65,
            "commitSha is too_long (limit 256)",
        ),
        (
            commit(&["--commit-sha", SHA]),
            65,
            "repo is required_for_commit",
        ),
        (
            commit(&["--repo", "someone/else", "--commit-sha", SHA]),
            70,
            "not on the workspace's GitHub App allowlist",
        ),
        (
            owned(&[
                "project",
                "evidence",
                "add",
                PROJECT,
                "--kind",
                "screenshot",
                "--source",
                "x",
            ]),
            64,
            "kind is invalid",
        ),
        (
            owned(&[
                "project",
                "evidence",
                "add",
                PROJECT,
                "--kind",
                "external_url",
                "--source",
                "x",
                "--work-item",
                "b0000000-0000-4000-8000-000000000099",
            ]),
            64,
            "Work item not found",
        ),
        (
            owned(&[
                "project",
                "evidence",
                "add",
                "00000000-0000-4000-8000-000000000000",
                "--kind",
                "external_url",
                "--source",
                "x",
            ]),
            64,
            "Project not found",
        ),
        (
            owned(&[
                "project",
                "evidence",
                "add",
                PROJECT,
                "--kind",
                "external_url",
                "--source",
                "x",
                "--metadata",
                "[1]",
            ]),
            64,
            "metadata is invalid",
        ),
        (
            owned(&["project", "evidence", "list", PROJECT, "--trust", "nope"]),
            64,
            "trust is invalid",
        ),
        (
            owned(&["project", "evidence", "list", PROJECT, "--limit", "101"]),
            65,
            "limit is too_large (limit 100)",
        ),
    ];
    for (args, code, message) in edges {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (got, stderr) = refused(&home, &args);
        assert_eq!(got, code, "{args:?}: {stderr}");
        assert!(stderr.contains(message), "{args:?}: {stderr}");
        assert_eq!(counts(&home), [0, 0, 0], "{args:?}");
    }
    list(&home, &["--limit", "100"]);
}

#[test]
fn tty_views_print_an_empty_page_and_an_attached_record() {
    let home = home();
    let empty = home.run_ok(&["project", "evidence", "list", PROJECT]);
    assert_eq!(empty, "no evidence\n");
    let shown = home.run_ok(&[
        "project",
        "evidence",
        "add",
        PROJECT,
        "--kind",
        "deployment",
        "--source",
        "cd",
        "--work-item",
        ITEM,
    ]);
    let lines: Vec<&str> = shown.lines().collect();
    assert!(lines[0].starts_with("id            "), "{shown}");
    for expected in [
        "kind          deployment",
        "trust         self_reported",
        &format!("work item     {ITEM}"),
        "source        cd",
        "external id   -",
        "url           -",
        "creator       user:local-operator",
    ] {
        assert!(
            lines.contains(&expected),
            "{expected:?} missing from {shown}"
        );
    }
}
