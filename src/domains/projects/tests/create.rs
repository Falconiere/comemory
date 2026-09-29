#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! Test mirror for `src/domains/projects/create.rs`, against a real temp data
//! directory: the draft charter and its view, exactly one `project.created`
//! event plus one `activity_log` row, and — for every refusal, including a
//! failure after the project row is written — no row left behind.

use comemory::config::{Config, Paths};
use comemory::domains::projects::authority::{self, Envelope};
use comemory::domains::projects::create::{self, Request};
use comemory::errors::{Error, Result};
use comemory::store::{Connection, connection};
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};

/// A migrated data directory and its open connection.
struct Home {
    _dir: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    conn: Connection,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure_dirs().unwrap();
        let conn = connection::open(paths.db_path()).unwrap();
        Self {
            _dir: dir,
            paths,
            cfg: Config::defaults(),
            conn,
        }
    }

    fn create(&mut self, req: Request) -> Result<create::Response> {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        authority::run(&mut ctx, &Envelope::local_operator(), req)
    }

    fn count(&self, table: &str) -> i64 {
        self.conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// Row counts of every table a create writes.
    fn counts(&self) -> [i64; 4] {
        [
            "projects",
            "project_repositories",
            "project_criteria",
            "project_activity_events",
        ]
        .map(|t| self.count(t))
    }
}

fn request(name: &str, key_prefix: &str) -> Request {
    Request {
        id: None,
        workspace_id: None,
        name: name.to_string(),
        key_prefix: key_prefix.to_string(),
        outcome: "Ship the loop".to_string(),
        success_criteria: vec!["Tests pass".to_string(), "Docs ship".to_string()],
        constraints: vec!["Offline first".to_string()],
        non_goals: vec!["A mesh".to_string()],
        repositories: vec![
            " Falconiere/Comemory.git ".to_string(),
            "falconiere/comemory".to_string(),
        ],
        lead_user_id: None,
        target_date: Some("2026-10-01".to_string()),
    }
}

/// The refusal's class and its serialized details.
fn refusal(result: Result<create::Response>) -> (Class, String, String) {
    let e = result.expect_err("expected a refusal");
    let Error::Project(project) = &e else {
        panic!("not a project refusal: {e:?}")
    };
    (
        classify(&e).1,
        serde_json::to_string(&project.details()).unwrap(),
        e.to_string(),
    )
}

#[test]
fn creates_a_draft_charter_with_one_event_and_one_telemetry_row() {
    let mut home = Home::new();
    let mut req = request("Ship the Governed Delivery Loop!", "SHIP");
    req.id = Some("0F8C2D7E-3B1A-4C5D-9E6F-7A8B9C0D1E2F".to_string());
    req.workspace_id = Some(serde_json::json!("ignored"));
    let project = home.create(req).unwrap().project;

    assert_eq!(project.id, "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f");
    assert_eq!(project.slug, "ship-the-governed-delivery-loop");
    assert_eq!(
        (project.status.as_str(), project.health.as_str()),
        ("draft", "unknown")
    );
    assert_eq!((project.version, project.current_plan_version), (1, 0));
    assert_eq!(project.lead_user_id, "local-operator");
    assert_eq!(project.created_by, "local-operator");
    assert_eq!(
        project.target_date.as_deref(),
        Some("2026-10-01T00:00:00.000Z")
    );
    assert_eq!(
        project.repositories,
        vec!["falconiere/comemory".to_string()]
    );
    assert_eq!(project.constraints, vec!["Offline first".to_string()]);
    assert_eq!(project.non_goals, vec!["A mesh".to_string()]);
    let criteria: Vec<_> = project
        .criteria
        .iter()
        .map(|c| (c.position, c.description.as_str()))
        .collect();
    assert_eq!(criteria, vec![(0, "Tests pass"), (1, "Docs ship")]);
    assert!(project.created_at.ends_with('Z') && project.created_at == project.updated_at);
    let view = serde_json::to_value(&project).unwrap();
    assert!(view.get("workspaceId").is_none());
    assert_eq!(view["keyPrefix"], "SHIP");

    let (event_type, entity, actor, payload): (String, String, String, String) = home
        .conn
        .query_row(
            "SELECT event_type, entity_type || ':' || entity_id,
                    actor_principal_type || ':' || actor_principal_id, payload
             FROM project_activity_events",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(event_type, "project.created");
    assert_eq!(entity, format!("project:{}", project.id));
    assert_eq!(actor, "user:local-operator");
    assert_eq!(
        payload,
        r#"{"name":"Ship the Governed Delivery Loop!","keyPrefix":"SHIP","repositoryCount":1,"criteriaCount":2}"#
    );
    assert_eq!(home.counts(), [1, 1, 2, 1]);
    let telemetry: Vec<(String, i64)> = home
        .conn
        .prepare("SELECT command, ok FROM activity_log")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(telemetry, vec![("project.create".to_string(), 1)]);
}

#[test]
fn every_cap_accepts_its_limit_and_refuses_one_past_it_leaving_no_row() {
    type Edit = fn(&mut Request, usize);
    let cases: Vec<(&str, usize, Edit, &str, &str)> = vec![
        (
            "name",
            120,
            |r, n| r.name = "n".repeat(n),
            "name",
            "too_long",
        ),
        (
            "outcome",
            2000,
            |r, n| r.outcome = "o".repeat(n),
            "outcome",
            "too_long",
        ),
        (
            "keyPrefix",
            10,
            |r, n| r.key_prefix = "K".repeat(n),
            "keyPrefix",
            "too_long",
        ),
        (
            "successCriteria",
            50,
            |r, n| r.success_criteria = vec!["c".into(); n],
            "successCriteria",
            "too_many",
        ),
        (
            "criterion",
            500,
            |r, n| r.success_criteria = vec!["c".repeat(n)],
            "successCriteria.0",
            "too_long",
        ),
        (
            "constraints",
            50,
            |r, n| r.constraints = vec!["c".into(); n],
            "constraints",
            "too_many",
        ),
        (
            "constraint",
            500,
            |r, n| r.constraints = vec!["c".repeat(n)],
            "constraints.0",
            "too_long",
        ),
        (
            "nonGoals",
            50,
            |r, n| r.non_goals = vec!["g".into(); n],
            "nonGoals",
            "too_many",
        ),
        (
            "nonGoal",
            500,
            |r, n| r.non_goals = vec!["g".repeat(n)],
            "nonGoals.0",
            "too_long",
        ),
        (
            "repositories",
            50,
            |r, n| r.repositories = (0..n).map(|i| format!("o/r{i}")).collect(),
            "repositories",
            "too_many",
        ),
    ];
    for (label, limit, edit, field, reason) in cases {
        let mut home = Home::new();
        let mut at_limit = request("At limit", "AT");
        edit(&mut at_limit, limit);
        home.create(at_limit)
            .unwrap_or_else(|e| panic!("{label} at {limit} refused: {e}"));
        let before = home.counts();
        let mut over = request("Over limit", "OVER");
        edit(&mut over, limit + 1);
        let (class, details, message) = refusal(home.create(over));
        assert_eq!(class, Class::Unprocessable, "{label}");
        assert_eq!(
            details,
            format!(r#"{{"field":"{field}","reason":"{reason}","limit":{limit}}}"#),
            "{label}"
        );
        assert!(
            message.contains(&format!("(limit {limit})")),
            "{label}: {message}"
        );
        assert_eq!(home.counts(), before, "{label} left rows behind");
    }
}

#[test]
fn shape_refusals_are_invariants_and_a_bad_date_is_a_schema_edge() {
    let mut home = Home::new();
    let cases: Vec<(Request, &str, Class)> = vec![
        (
            Request {
                name: String::new(),
                ..request("x", "AA")
            },
            r#"{"field":"name","reason":"too_short","limit":1}"#,
            Class::Unprocessable,
        ),
        (
            request("x", "1A"),
            r#"{"field":"keyPrefix","reason":"invalid_format"}"#,
            Class::Unprocessable,
        ),
        (
            request("x", "A"),
            r#"{"field":"keyPrefix","reason":"too_short","limit":2}"#,
            Class::Unprocessable,
        ),
        (
            Request {
                id: Some("nope".into()),
                ..request("x", "AA")
            },
            r#"{"field":"id","reason":"invalid_format"}"#,
            Class::Unprocessable,
        ),
        (
            Request {
                repositories: vec!["ok/repo".into(), "not a repo".into()],
                ..request("x", "AA")
            },
            r#"{"field":"repositories.1","reason":"invalid_format"}"#,
            Class::Unprocessable,
        ),
        (
            Request {
                lead_user_id: Some(String::new()),
                ..request("x", "AA")
            },
            r#"{"field":"leadUserId","reason":"too_short","limit":1}"#,
            Class::Unprocessable,
        ),
        (
            Request {
                target_date: Some("next week".into()),
                ..request("x", "AA")
            },
            r#"{"field":"targetDate","reason":"invalid"}"#,
            Class::BadRequest,
        ),
        // Validation order: `name` is checked before `keyPrefix`.
        (
            Request {
                name: String::new(),
                ..request("x", "1A")
            },
            r#"{"field":"name","reason":"too_short","limit":1}"#,
            Class::Unprocessable,
        ),
    ];
    for (req, expected, class) in cases {
        let (got_class, details, _) = refusal(home.create(req));
        assert_eq!((got_class, details.as_str()), (class, expected));
    }
    assert_eq!(home.counts(), [0, 0, 0, 0]);
}

#[test]
fn duplicates_refuse_and_a_taken_slug_is_disambiguated() {
    let mut home = Home::new();
    let first = home.create(request("Same Name", "ONE")).unwrap().project;
    let second = home.create(request("Same Name", "TWO")).unwrap().project;
    assert_eq!(
        (first.slug.as_str(), second.slug.as_str()),
        ("same-name", "same-name-2")
    );
    let before = home.counts();

    let (class, details, message) = refusal(home.create(request("Other", "ONE")));
    assert_eq!(class, Class::Unprocessable);
    assert_eq!(details, r#"{"field":"keyPrefix","reason":"duplicate"}"#);
    assert_eq!(message, "keyPrefix is already used in this workspace");
    let (_, details, _) = refusal(home.create(Request {
        id: Some(first.id.clone()),
        ..request("Other", "THREE")
    }));
    assert_eq!(details, r#"{"field":"id","reason":"duplicate"}"#);
    assert_eq!(home.counts(), before);
}

#[test]
fn a_failure_after_the_project_row_rolls_everything_back() {
    let mut home = Home::new();
    home.conn
        .execute_batch(
            "CREATE TRIGGER refuse_event BEFORE INSERT ON project_activity_events
             BEGIN SELECT RAISE(ABORT, 'event refused'); END;",
        )
        .unwrap();
    let e = home.create(request("Late failure", "LATE")).unwrap_err();
    assert!(e.to_string().contains("event refused"), "{e}");
    assert_eq!(home.counts(), [0, 0, 0, 0]);
    let failed: i64 = home
        .conn
        .query_row(
            "SELECT COUNT(*) FROM activity_log WHERE command = 'project.create' AND ok = 0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(failed, 1, "the failed run still gets its telemetry row");
}
