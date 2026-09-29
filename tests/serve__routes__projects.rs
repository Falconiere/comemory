#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! `/api/v1/projects` through a real `comemory serve` on a fresh data
//! directory (#326): `POST` answers `201` with the charter, `GET` by id and
//! the list return the same view, every limit answers `422 invalid_request`
//! with `{field, reason, limit}`, a malformed body, query or cursor answers
//! `400`, and a read-only server refuses the create.

use serde_json::{Value, json};

#[path = "common/serve_bin.rs"]
mod serve_bin;

use serve_bin::ServeHome;

fn charter(key: &str) -> Value {
    json!({
        "workspaceId": "ignored-by-the-engine",
        "name": format!("Project {key}"),
        "keyPrefix": key,
        "outcome": "Ship it",
        "successCriteria": ["Tests pass"],
        "repositories": ["Falconiere/comemory"],
    })
}

#[test]
fn create_show_and_list_agree_and_create_answers_201() {
    let home = ServeHome::new();
    let (status, body) = home.post_raw("/projects", &charter("SHIP"));
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["meta"]["command"], "project.create");
    let project = body["data"]["project"].clone();
    assert_eq!(project["status"], "draft");
    assert_eq!(project["repositories"], json!(["falconiere/comemory"]));
    let id = project["id"].as_str().unwrap();

    let shown = home.get(&format!("/projects/{id}"));
    assert_eq!(shown["project"], project);
    let listed = home.get("/projects");
    assert_eq!(listed["projects"], json!([project]));
    assert_eq!(listed["nextCursor"], Value::Null);

    let (status, body) = home.get_raw("/projects/00000000-0000-4000-8000-000000000000");
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"]["code"], "project_not_found");
}

#[test]
fn limits_answer_422_and_malformed_input_answers_400() {
    let home = ServeHome::new();
    let mut long = charter("LONG");
    long["name"] = json!("n".repeat(121));
    let (status, body) = home.post_raw("/projects", &long);
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["code"], "invalid_request");
    assert_eq!(
        body["error"]["details"],
        json!({"field": "name", "reason": "too_long", "limit": 120})
    );

    let mut many = charter("MANY");
    many["successCriteria"] = json!(vec!["c"; 51]);
    let (status, body) = home.post_raw("/projects", &many);
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        body["error"]["details"],
        json!({"field": "successCriteria", "reason": "too_many", "limit": 50})
    );

    let (status, body) = home.get_q_raw("/projects", &[("limit", "101")]);
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["details"]["limit"], 100);

    let (status, body) = home.get_q_raw("/projects", &[("cursor", "abc")]);
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["code"], "invalid_request");
    assert_eq!(body["error"]["message"], "cursor is invalid");

    let (status, body) = home.post_text_raw("/projects", "[1,2]".to_string());
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["message"], "body must be an object");

    let (status, body) = home.post_raw("/projects", &json!({"keyPrefix": "AB", "outcome": "o"}));
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"]["details"],
        json!({"field": "name", "reason": "required"})
    );

    let (status, body) = home.get_raw("/projects/not-a-uuid");
    assert_eq!(status, 400, "{body}");

    home.post_raw("/projects", &charter("SHIP"));
    let (status, body) = home.post_raw("/projects", &charter("SHIP"));
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        body["error"]["details"],
        json!({"field": "keyPrefix", "reason": "duplicate"})
    );
    let listed = home.get("/projects");
    assert_eq!(
        listed["projects"].as_array().unwrap().len(),
        1,
        "a refused create left a row"
    );
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
