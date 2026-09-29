#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `comemory project plan show` through the real binary (#335): a freshly
//! chartered project reads plan version 0 with empty collections, or with
//! only its charter's success criteria when it has some; the plan
//! `tests/fixtures/projects/plan_seed.sql` writes straight into the store
//! renders both criteria levels and every live dependency, with the archived
//! milestone, item, criterion and their edges absent, in `--json` and on the
//! TTY; a malformed or unknown id exits 64 naming the refusal.

use serde_json::{Value, json};

#[path = "common/cli_bin.rs"]
mod cli_bin;

use cli_bin::CliHome;

/// The project `plan_seed.sql` writes its plan into.
const PROJECT: &str = "11111111-1111-4111-8111-111111111111";

/// A data directory holding [`PROJECT`], chartered through the CLI.
fn chartered() -> CliHome {
    let home = CliHome::new();
    home.run_json(&[
        "project",
        "create",
        "--id",
        PROJECT,
        "--name",
        "Plan",
        "--key-prefix",
        "PLAN",
        "--outcome",
        "A plan reads back",
    ]);
    home
}

/// `--json project plan show <id>`'s plan.
fn plan(home: &CliHome, id: &str) -> Value {
    home.run_json(&["project", "plan", "show", id])["plan"].clone()
}

/// The last two digits of every `key` id in `list`.
fn tails(list: &Value, key: &str) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|entry| entry[key].as_str().unwrap()[34..].to_string())
        .collect()
}

#[test]
fn a_fresh_project_reads_plan_version_zero_with_empty_collections() {
    let home = chartered();
    assert_eq!(
        plan(&home, PROJECT),
        json!({
            "projectId": PROJECT, "planVersion": 0,
            "milestones": [], "workItems": [], "criteria": [], "dependencies": []
        })
    );
    let tty = home.run_ok(&["project", "plan", "show", PROJECT]);
    assert!(
        tty.contains(&format!("plan          v0 of {PROJECT}")),
        "{tty}"
    );
    assert!(tty.contains("no committed plan entities"), "{tty}");
}

#[test]
fn a_charter_s_success_criteria_are_the_plan_s_criteria_at_version_zero() {
    let home = CliHome::new();
    let created = home.run_json(&[
        "project",
        "create",
        "--name",
        "Criteria",
        "--key-prefix",
        "CRIT",
        "--outcome",
        "Criteria read back",
        "--success-criterion",
        "The plan lists me",
    ]);
    let id = created["project"]["id"].as_str().unwrap();
    let plan = plan(&home, id);
    assert_eq!(plan["planVersion"], 0);
    assert_eq!(
        plan["criteria"][0]["id"],
        created["project"]["criteria"][0]["id"]
    );
    assert_eq!(plan["criteria"][0]["workItemId"], Value::Null);
    assert_eq!(plan["milestones"], json!([]));
    assert_eq!(plan["workItems"], json!([]));

    let tty = home.run_ok(&["project", "plan", "show", id]);
    assert!(tty.contains("[open] The plan lists me  (project)"), "{tty}");
    assert!(!tty.contains("no committed plan entities"), "{tty}");
}

#[test]
fn a_seeded_plan_renders_both_criteria_levels_and_every_live_dependency() {
    let home = chartered();
    rusqlite::Connection::open(home.data_dir().join("comemory.db"))
        .unwrap()
        .execute_batch(include_str!("fixtures/projects/plan_seed.sql"))
        .unwrap();

    let plan = plan(&home, PROJECT);
    assert_eq!(plan["planVersion"], 3);
    assert_eq!(tails(&plan["milestones"], "id"), ["01", "02"]);
    assert_eq!(tails(&plan["workItems"], "id"), ["02", "04", "01", "05"]);
    assert_eq!(tails(&plan["criteria"], "id"), ["01", "02", "04"]);
    assert_eq!(plan["criteria"][0]["workItemId"], Value::Null);
    assert_eq!(
        plan["criteria"][1]["workItemId"],
        "b0000000-0000-4000-8000-000000000002"
    );
    assert_eq!(tails(&plan["dependencies"], "blockerId"), ["01", "04"]);
    assert_eq!(tails(&plan["dependencies"], "blockedId"), ["02", "02"]);

    let tty = home.run_ok(&["project", "plan", "show", PROJECT]);
    assert!(tty.contains("plan          v3 of"), "{tty}");
    assert!(tty.contains("#2 [ready] Render the items"), "{tty}");
    assert!(
        tty.contains("[open] Plans read offline  (project)"),
        "{tty}"
    );
    assert!(
        !tty.contains("Dropped"),
        "an archived entity rendered: {tty}"
    );
    assert_eq!(tty.matches("blocks        ").count(), 2, "{tty}");
}

#[test]
fn a_malformed_or_unknown_id_exits_64_naming_the_refusal() {
    let home = chartered();
    for (id, message) in [
        ("not-a-uuid", "projectId is invalid"),
        ("00000000-0000-4000-8000-000000000000", "Project not found"),
    ] {
        let out = home
            .bin()
            .args(["project", "plan", "show", id])
            .output()
            .unwrap();
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert_eq!(out.status.code(), Some(64), "{stderr}");
        assert!(stderr.contains(message), "{stderr}");
    }
}
