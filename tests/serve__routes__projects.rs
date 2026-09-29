#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! `/api/v1/projects` through a real `comemory serve` on a fresh data
//! directory. Local-mode HTTP runs as the local agent (#315): it reads with
//! `project.read` but holds no human verb, so `POST` answers `403
//! project_agent_scope` and writes nothing — not even for a charter that would
//! breach a limit — while a project the CLI (the local operator) chartered in
//! the same data directory reads back through both `GET` routes. Malformed
//! input — a create without `idempotencyKey` included — still answers `400`,
//! a list limit `422`, and a read-only server refuses the create with `405`
//! before authority is consulted.

use assert_cmd::Command;
use serde_json::{Value, json};

#[path = "common/serve_bin.rs"]
mod serve_bin;

use serve_bin::ServeHome;

fn charter(key: &str) -> Value {
    json!({
        "workspaceId": "ignored-by-the-engine",
        "idempotencyKey": format!("create-{key}"),
        "name": format!("Project {key}"),
        "keyPrefix": key,
        "outcome": "Ship it",
        "successCriteria": ["Tests pass"],
        "repositories": ["Falconiere/comemory"],
    })
}

/// Charter `key` through the real CLI in the server's data directory, as the
/// local operator; the created project's view.
fn seed(home: &ServeHome, key: &str) -> Value {
    let out = Command::cargo_bin("comemory")
        .unwrap()
        .env("COMEMORY_DATA_DIR", home.data_dir())
        .args(["--json", "project", "create", "--name", key])
        .args(["--key-prefix", key, "--outcome", "Ship it"])
        .args(["--repository", "Falconiere/comemory"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let created: Value = serde_json::from_slice(&out.stdout).unwrap();
    created["project"].clone()
}

/// `[projects, project_activity_events, activity_log]` rows, read straight
/// from the server's database.
fn rows(home: &ServeHome) -> [i64; 3] {
    let db = rusqlite::Connection::open_with_flags(
        home.data_dir().join("comemory.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    [
        "SELECT COUNT(*) FROM projects",
        "SELECT COUNT(*) FROM project_activity_events",
        "SELECT COUNT(*) FROM activity_log",
    ]
    .map(|sql| db.query_row(sql, [], |r| r.get(0)).unwrap())
}

#[test]
fn the_local_agent_is_refused_the_create_and_writes_nothing() {
    let home = ServeHome::new();
    let (status, body) = home.post_raw("/projects", &charter("SHIP"));
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["meta"]["command"], "project.create");
    assert_eq!(
        body["error"],
        json!({
            "code": "project_agent_scope",
            "message": "This command requires a signed-in human",
            "details": {"code": "project_agent_scope"},
        })
    );

    // Authority comes before validation: an over-limit charter is refused
    // for who sent it, not for its name.
    let mut long = charter("LONG");
    long["name"] = json!("n".repeat(121));
    let (status, body) = home.post_raw("/projects", &long);
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error"]["code"], "project_agent_scope");

    assert_eq!(rows(&home), [0, 0, 0], "a refused create wrote a row");
    assert_eq!(home.get("/projects")["projects"], json!([]));
}

#[test]
fn the_agent_reads_back_what_the_operator_chartered() {
    let home = ServeHome::new();
    let project = seed(&home, "SHIP");
    assert_eq!(project["createdBy"], "local-operator");
    assert_eq!(project["repositories"], json!(["falconiere/comemory"]));
    let id = project["id"].as_str().unwrap();

    let shown = home.get(&format!("/projects/{id}"));
    assert_eq!(shown["project"], project);
    let listed = home.get("/projects");
    assert_eq!(listed["projects"], json!([project]));
    assert_eq!(listed["nextCursor"], Value::Null);
    assert_eq!(rows(&home), [1, 1, 1]);

    let (status, body) = home.get_raw("/projects/00000000-0000-4000-8000-000000000000");
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"]["code"], "project_not_found");
}

/// Run `comemory --json <args>` in the server's data directory, as the local
/// operator; its parsed stdout.
fn cli(home: &ServeHome, args: &[&str]) -> Value {
    let out = Command::cargo_bin("comemory")
        .unwrap()
        .env("COMEMORY_DATA_DIR", home.data_dir())
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn the_agent_reads_the_plan_the_store_holds() {
    const PROJECT: &str = "11111111-1111-4111-8111-111111111111";
    let home = ServeHome::new();
    cli(
        &home,
        &["project", "create", "--id", PROJECT, "--name", "Plan"]
            .into_iter()
            .chain(["--key-prefix", "PLAN", "--outcome", "o"])
            .collect::<Vec<_>>(),
    );
    let plan = home.get(&format!("/projects/{PROJECT}/plan"));
    assert_eq!(
        plan,
        json!({"plan": {
            "projectId": PROJECT, "planVersion": 0,
            "milestones": [], "workItems": [], "criteria": [], "dependencies": []
        }})
    );

    rusqlite::Connection::open(home.data_dir().join("comemory.db"))
        .unwrap()
        .execute_batch(include_str!("fixtures/projects/plan_seed.sql"))
        .unwrap();
    let plan = home.get(&format!("/projects/{}/plan", PROJECT.to_uppercase()));
    // HTTP re-keys the body, so the CLI's plan compares as a value.
    assert_eq!(plan, cli(&home, &["project", "plan", "show", PROJECT]));
    let plan = &plan["plan"];
    assert_eq!(plan["planVersion"], 3);
    let live: Vec<&str> = plan["workItems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        live,
        [
            "Render the items",
            "Fix the edge",
            "Build the reader",
            "Outlive the milestone"
        ]
    );
    assert_eq!(plan["milestones"].as_array().unwrap().len(), 2);
    assert_eq!(plan["criteria"].as_array().unwrap().len(), 3);
    let archived = "b0000000-0000-4000-8000-000000000003";
    let edges = plan["dependencies"].as_array().unwrap();
    assert_eq!(edges.len(), 2);
    assert!(
        edges
            .iter()
            .all(|e| e["blockerId"] != archived && e["blockedId"] != archived)
    );

    let (status, body) = home.get_raw("/projects/not-a-uuid/plan");
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["code"], "invalid_request");
    assert_eq!(body["error"]["message"], "projectId is invalid");
    let (status, body) = home.get_raw("/projects/00000000-0000-4000-8000-000000000000/plan");
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"]["code"], "project_not_found");
    assert_eq!(rows(&home), [1, 1, 1], "a plan read wrote a row");
}

#[test]
fn malformed_input_answers_400_and_a_list_limit_422() {
    let home = ServeHome::new();
    let (status, body) = home.get_q_raw("/projects", &[("limit", "101")]);
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        body["error"]["details"],
        json!({"field": "limit", "reason": "too_large", "limit": 100})
    );

    let (status, body) = home.get_q_raw("/projects", &[("cursor", "abc")]);
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["code"], "invalid_request");
    assert_eq!(body["error"]["message"], "cursor is invalid");

    // The body is parsed before the core runs, so it answers `400` even
    // for a caller the core would refuse.
    let (status, body) = home.post_text_raw("/projects", "[1,2]".to_string());
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["message"], "body must be an object");

    let (status, body) = home.post_raw(
        "/projects",
        &json!({"idempotencyKey": "k1", "keyPrefix": "AB", "outcome": "o"}),
    );
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"]["details"],
        json!({"field": "name", "reason": "required"})
    );

    // Every mutation names its retry key (#327): a create without one is
    // malformed, whoever sends it.
    let mut unkeyed = charter("KEYLESS");
    unkeyed.as_object_mut().unwrap().remove("idempotencyKey");
    let (status, body) = home.post_raw("/projects", &unkeyed);
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["code"], "invalid_request");
    assert_eq!(body["error"]["message"], "idempotencyKey is required");
    assert_eq!(
        body["error"]["details"],
        json!({"field": "idempotencyKey", "reason": "required"})
    );

    let (status, body) = home.get_raw("/projects/not-a-uuid");
    assert_eq!(status, 400, "{body}");
    assert_eq!(rows(&home), [0, 0, 0]);
}

#[test]
fn a_read_only_server_refuses_the_create_but_still_lists() {
    let home = ServeHome::with_args(&["--read-only"]);
    let (status, body) = home.post_raw("/projects", &charter("SHIP"));
    assert_eq!(status, 405, "{body}");
    assert_eq!(body["error"]["code"], "read_only");
    let listed = home.get("/projects");
    assert_eq!(listed["projects"], json!([]));
}

/// A refused read: `(route, query, status, details)`.
type Refusal<'a> = (&'a str, &'a [(&'a str, &'a str)], u16, Value);

#[test]
fn the_activity_page_matches_the_cli_and_refuses_by_edge() {
    let home = ServeHome::new();
    let project = seed(&home, "WALK");
    let id = project["id"].as_str().unwrap();
    let out = Command::cargo_bin("comemory")
        .unwrap()
        .env("COMEMORY_DATA_DIR", home.data_dir())
        .args(["--json", "project", "activity", id, "--order", "asc"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let cli: Value = serde_json::from_slice(&out.stdout).unwrap();

    let path = format!("/projects/{id}/activity");
    let page = home.get_q(&path, &[("order", "asc"), ("workspaceId", "w")]);
    assert_eq!(page, cli);
    assert_eq!(page["events"][0]["eventType"], "project.created");
    assert_eq!(page["events"][0]["actorPrincipalId"], "local-operator");
    assert_eq!(page["nextCursor"], Value::Null);
    let before = rows(&home);

    let refusals: [Refusal<'_>; 7] = [
        (
            &path,
            &[("limit", "201")],
            422,
            json!({"field": "limit", "reason": "too_large", "limit": 200}),
        ),
        (
            &path,
            &[("limit", "0")],
            422,
            json!({"field": "limit", "reason": "too_small", "limit": 1}),
        ),
        (
            &path,
            &[("limit", "ten")],
            400,
            json!({"field": "limit", "reason": "invalid"}),
        ),
        (
            &path,
            &[("cursor", "abc")],
            400,
            json!({"field": "cursor", "reason": "invalid"}),
        ),
        (
            &path,
            &[("order", "sideways")],
            400,
            json!({"field": "order", "reason": "invalid"}),
        ),
        (
            &path,
            &[("status", "active")],
            400,
            json!({"field": "status", "reason": "invalid"}),
        ),
        (
            "/projects/not-a-uuid/activity",
            &[],
            400,
            json!({"field": "projectId", "reason": "invalid"}),
        ),
    ];
    for (route, query, status, details) in refusals {
        let (got, body) = home.get_q_raw(route, query);
        assert_eq!(got, status, "{route} {query:?}: {body}");
        assert_eq!(body["error"]["code"], "invalid_request", "{body}");
        assert_eq!(body["error"]["details"], details, "{body}");
    }
    let (status, body) = home.get_raw("/projects/00000000-0000-4000-8000-000000000000/activity");
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"]["code"], "project_not_found");
    assert_eq!(rows(&home)[..2], before[..2], "a read wrote a row");
}
