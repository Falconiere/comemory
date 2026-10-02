#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! `comemory project proposal submit|list|show` through the real binary
//! (#336): the twelve-kind fixture submitted from a file is stored
//! normalized and shown back equal, the draft project reads `planning`, a
//! second proposal from stdin lists beside it under `pending`, and a replay
//! prints the first answer; each refusal — a stale base, a patch identity,
//! an archived project, an unknown proposal, a malformed operations flag —
//! exits with its class's code naming the refusal.

use serde_json::{Value, json};

#[path = "common/cli_bin.rs"]
mod cli_bin;

use cli_bin::CliHome;

const PROJECT: &str = "11111111-1111-4111-8111-111111111111";

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/projects/proposal_operations.json")).unwrap()
}

/// A data directory holding the draft [`PROJECT`].
fn chartered() -> CliHome {
    let home = CliHome::new();
    home.run_json(&[
        "project",
        "create",
        "--id",
        PROJECT,
        "--name",
        "Proposals",
        "--key-prefix",
        "PROP",
        "--outcome",
        "Plans change by review",
    ]);
    home
}

/// `project proposal submit` with `extra` flags after the common ones.
fn submit_args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "--json",
        "project",
        "proposal",
        "submit",
        PROJECT,
        "--base-plan-version",
        "0",
        "--rationale",
        "Scope the reader",
        "--assumption",
        "The reader is merged",
        "--risk",
        "Churn",
    ];
    args.extend_from_slice(extra);
    args
}

/// Run to failure; `(exit code, stderr)`.
fn refused(home: &CliHome, args: &[&str]) -> (i32, String) {
    let out = home.bin().args(args).output().unwrap();
    assert!(!out.status.success(), "{args:?} unexpectedly succeeded");
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn a_file_proposal_round_trips_moves_draft_to_planning_and_lists_beside_a_stdin_one() {
    let home = chartered();
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ops.json");
    std::fs::write(&file, fixture["operations"].to_string()).unwrap();
    let file = file.to_str().unwrap();

    let args = submit_args(&["--operations-file", file, "--idempotency-key", "k1"]);
    let first = home.run_ok(&args);
    let proposal: Value = serde_json::from_str(first.trim()).unwrap();
    let proposal = &proposal["proposal"];
    assert_eq!(proposal["operations"], fixture["normalized"]);
    assert_eq!(
        (
            &proposal["state"],
            &proposal["assumptions"],
            &proposal["risks"]
        ),
        (
            &json!("pending"),
            &json!(["The reader is merged"]),
            &json!(["Churn"])
        )
    );
    assert_eq!(proposal["proposerPrincipalId"], "local-operator");
    assert_eq!(
        home.run_ok(&args),
        first,
        "a replay prints the first answer"
    );

    let shown = home.run_json(&["project", "show", PROJECT]);
    assert_eq!(shown["project"]["status"], "planning");
    let id = proposal["id"].as_str().unwrap();
    let show = home.run_json(&["project", "proposal", "show", PROJECT, id]);
    assert_eq!(&show["proposal"], proposal);

    let archive =
        json!([{"op": "work_item.archive", "workItemId": "b0000000-0000-4000-8000-000000000009"}]);
    let out = home
        .bin()
        .args(submit_args(&["--operations-file", "-"]))
        .write_stdin(archive.to_string())
        .output()
        .unwrap();
    assert!(out.status.success());
    let second: Value = serde_json::from_slice(&out.stdout).unwrap();
    let second_id = second["proposal"]["id"].as_str().unwrap();

    let page = home.run_json(&["project", "proposal", "list", PROJECT, "--state", "pending"]);
    let ids: Vec<&str> = page["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [second_id, id]);
    let one = home.run_json(&["project", "proposal", "list", PROJECT, "--limit", "1"]);
    let cursor = one["nextCursor"].as_str().unwrap();
    let rest = home.run_json(&["project", "proposal", "list", PROJECT, "--cursor", cursor]);
    assert_eq!(rest["proposals"][0]["id"], id);
    assert_eq!(rest["nextCursor"], Value::Null);

    let tty = home.run_ok(&["project", "proposal", "list", PROJECT]);
    assert!(tty.contains("pending"), "{tty}");
    let tty = home.run_ok(&["project", "proposal", "show", PROJECT, id]);
    assert!(tty.contains(&format!("proposal      {id}")), "{tty}");
    assert!(
        tty.contains(r#"operation     {"op":"dependency.remove""#),
        "{tty}"
    );
    assert!(tty.contains("risk          Churn"), "{tty}");
}

#[test]
fn each_refusal_exits_with_its_class_naming_the_refusal() {
    let home = chartered();
    let id = "b0000000-0000-4000-8000-000000000001";
    let archive = json!([{"op": "work_item.archive", "workItemId": id}]).to_string();
    let patch =
        json!([{"op": "work_item.update", "workItemId": id, "patch": {"id": id}}]).to_string();
    let over: Vec<Value> = (0..201)
        .map(|_| json!({"op": "work_item.archive", "workItemId": id}))
        .collect();
    let over = Value::from(over).to_string();

    let mut stale = submit_args(&["--operations", &archive]);
    stale[6] = "4";
    let (code, stderr) = refused(&home, &stale);
    assert_eq!(code, 75, "{stderr}");
    assert!(
        stderr.contains("The plan has changed since this proposal was written"),
        "{stderr}"
    );

    let (code, stderr) = refused(&home, &submit_args(&["--operations", &patch]));
    assert_eq!(code, 64, "{stderr}");
    assert!(stderr.contains("operations.0 is invalid"), "{stderr}");

    let (code, stderr) = refused(&home, &submit_args(&["--operations", &over]));
    assert_eq!(code, 65, "{stderr}");
    assert!(
        stderr.contains("This proposal exceeds the operations limit of 200"),
        "{stderr}"
    );

    let (code, stderr) = refused(&home, &submit_args(&["--operations", "not json"]));
    assert_eq!(code, 64, "{stderr}");
    assert!(stderr.contains("operations is invalid"), "{stderr}");

    // Past the 2,000-operation schema bound: the schema edge's 400, not a cap.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ops.json");
    let schema_over: Vec<Value> = (0..2001)
        .map(|_| json!({"op": "work_item.archive", "workItemId": id}))
        .collect();
    std::fs::write(&file, Value::from(schema_over).to_string()).unwrap();
    let file = file.to_str().unwrap();
    let (code, stderr) = refused(&home, &submit_args(&["--operations-file", file]));
    assert_eq!(code, 64, "{stderr}");
    assert!(stderr.contains("operations is invalid"), "{stderr}");

    let unknown = "33333333-3333-4333-8333-333333333333";
    let (code, stderr) = refused(&home, &["project", "proposal", "show", PROJECT, unknown]);
    assert_eq!(code, 64, "{stderr}");
    assert!(stderr.contains("Proposal not found"), "{stderr}");

    let db = rusqlite::Connection::open(home.data_dir().join("comemory.db")).unwrap();
    db.execute("UPDATE projects SET archived_at = 1", [])
        .unwrap();
    let (code, stderr) = refused(&home, &submit_args(&["--operations", &archive]));
    assert_eq!(code, 75, "{stderr}");
    assert!(stderr.contains("This project is archived"), "{stderr}");
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM project_plan_proposals", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0, "a refused submission wrote a proposal");
}
