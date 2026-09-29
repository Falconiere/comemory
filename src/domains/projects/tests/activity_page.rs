#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/activity_page.rs`: a created
//! project's page is its one `project.created` event in the platform's view,
//! key for key; a walk in either order is total and each is the other
//! reversed, with the `limit + 1` probe ending an exact remainder with a
//! `null` cursor; every refusal takes its edge, and an agent without
//! `project.read` is refused before the store is opened.

use comemory::config::{Config, Paths};
use comemory::domains::projects::activity_page::{Request, Response, view};
use comemory::domains::projects::authority::{self, Capabilities, Envelope};
use comemory::domains::projects::create;
use comemory::store::project_activity::{self, ActivityRow, NewProjectEvent};
use comemory::store::{Connection, connection};
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};
use serde_json::json;

/// A data directory holding one created project.
struct Home {
    _dir: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    conn: Connection,
    project: String,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure_dirs().unwrap();
        let cfg = Config::defaults();
        let mut conn = connection::open(paths.db_path()).unwrap();
        let req: create::Request = serde_json::from_value(json!({
            "idempotencyKey": "k1", "name": "Walk me", "keyPrefix": "WALK", "outcome": "Walked"
        }))
        .unwrap();
        let project = {
            let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
            authority::run(&mut ctx, &Envelope::local_operator(), req)
                .unwrap()
                .project
                .id
        };
        Self {
            _dir: dir,
            paths,
            cfg,
            conn,
            project,
        }
    }

    /// `n` more events on the project, all in one millisecond.
    fn add_events(&self, n: usize) {
        for k in 0..n {
            project_activity::insert(
                &self.conn,
                &NewProjectEvent {
                    id: &format!("{k:08x}-1111-4000-8000-000000000000"),
                    project_id: &self.project,
                    actor_type: "project_agent",
                    actor_id: "local-agent",
                    event_type: "project.health_reported",
                    entity_type: "project",
                    entity_id: &self.project,
                    payload: "{}",
                    at_ms: 2_000_000_000_000,
                },
            )
            .unwrap();
        }
    }

    fn page(&mut self, req: Request) -> comemory::errors::Result<Response> {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        authority::run(&mut ctx, &Envelope::local_operator(), req)
    }

    fn request(&self) -> Request {
        Request {
            id: self.project.clone(),
            ..Request::default()
        }
    }

    /// Every event id in `order`, paged `limit` at a time.
    fn walk(&mut self, order: &str, limit: i64) -> Vec<String> {
        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let page = self
                .page(Request {
                    limit: Some(limit),
                    cursor,
                    order: Some(order.into()),
                    ..self.request()
                })
                .unwrap();
            assert!(page.events.len() as i64 <= limit);
            seen.extend(page.events.into_iter().map(|e| e.id));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return seen,
            }
        }
    }
}

#[test]
fn a_page_is_the_platform_view_key_for_key() {
    let mut home = Home::new();
    let page = home.page(home.request()).unwrap();
    assert_eq!(page.next_cursor, None);
    // The serialized text keeps the platform's key order; a `Value` would sort it.
    let text = serde_json::to_string(&page.events[0]).unwrap();
    let keys = [
        "id",
        "projectId",
        "actorPrincipalType",
        "actorPrincipalId",
        "eventType",
        "entityType",
        "entityId",
        "payload",
        "createdAt",
    ];
    let at: Vec<usize> = keys
        .iter()
        .map(|k| text.find(&format!("\"{k}\":")).unwrap())
        .collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "key order: {text}");
    let wire = serde_json::to_value(&page).unwrap();
    let event = &wire["events"][0];
    assert_eq!(event.as_object().unwrap().len(), keys.len());
    assert_eq!(wire["events"].as_array().unwrap().len(), 1);
    assert_eq!(event["projectId"], json!(home.project));
    assert_eq!(event["actorPrincipalType"], "user");
    assert_eq!(event["actorPrincipalId"], "local-operator");
    assert_eq!(event["eventType"], "project.created");
    assert_eq!(event["entityType"], "project");
    assert_eq!(event["entityId"], json!(home.project));
    assert!(event["payload"].is_object());
    let created = event["createdAt"].as_str().unwrap();
    assert!(
        created.len() == 24 && created.ends_with('Z'),
        "not toISOString(): {created}"
    );
    // The upper-case id reads the same page.
    let upper = home
        .page(Request {
            id: home.project.to_uppercase(),
            ..Request::default()
        })
        .unwrap();
    assert_eq!(serde_json::to_value(&upper).unwrap(), wire);
}

#[test]
fn both_orders_are_total_and_each_is_the_other_reversed() {
    let mut home = Home::new();
    home.add_events(9);
    let every = home.walk("desc", 200);
    assert_eq!(every.len(), 10);
    // Newest first: the nine tied events by descending id, then the create.
    let mut tied = every[..9].to_vec();
    tied.sort();
    tied.reverse();
    assert_eq!(every[..9], tied[..]);
    for limit in 1..=11 {
        let desc = home.walk("desc", limit);
        let mut asc = home.walk("asc", limit);
        assert_eq!(desc, every, "desc at limit {limit}");
        asc.reverse();
        assert_eq!(asc, every, "asc reversed at limit {limit}");
    }
    // An exact remainder: the `limit + 1` probe ends the walk with no cursor.
    let whole = home
        .page(Request {
            limit: Some(10),
            ..home.request()
        })
        .unwrap();
    assert_eq!((whole.events.len(), whole.next_cursor), (10, None));
    // The default order is newest first, the default page 50.
    let default = home.page(home.request()).unwrap();
    let ids: Vec<String> = default.events.into_iter().map(|e| e.id).collect();
    assert_eq!(ids, every);
}

#[test]
fn every_refusal_takes_its_edge_and_writes_nothing() {
    let mut home = Home::new();
    let bad = |req: Request| (req, Class::BadRequest);
    let unprocessable = |req: Request| (req, Class::Unprocessable);
    let refusals = [
        bad(Request {
            id: "not-a-uuid".into(),
            ..Request::default()
        }),
        bad(Request {
            cursor: Some("abc".into()),
            ..home.request()
        }),
        bad(Request {
            order: Some("sideways".into()),
            ..home.request()
        }),
        unprocessable(Request {
            limit: Some(0),
            ..home.request()
        }),
        unprocessable(Request {
            limit: Some(201),
            ..home.request()
        }),
    ];
    for (req, class) in refusals {
        let e = home.page(req).unwrap_err();
        assert_eq!(classify(&e), ("invalid_request", class));
    }
    for limit in [1, 200] {
        home.page(Request {
            limit: Some(limit),
            ..home.request()
        })
        .unwrap();
    }
    let e = home
        .page(Request {
            id: "00000000-0000-4000-8000-000000000000".into(),
            ..Request::default()
        })
        .unwrap_err();
    assert_eq!(classify(&e), ("project_not_found", Class::NotFound));
    let events: i64 = home
        .conn
        .query_row("SELECT count(*) FROM project_activity_events", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(events, 1, "a read wrote an event");
}

#[test]
fn an_agent_without_project_read_is_refused_before_the_store_opens() {
    let home = Home::new();
    let writer = Envelope::agent("w", Capabilities::parse("t", &["evidence.create"]));
    let lazy = tempfile::tempdir().unwrap();
    let paths = Paths::new(lazy.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let e = authority::run(&mut ctx, &writer, home.request()).unwrap_err();
    assert_eq!(classify(&e), ("project_agent_scope", Class::Forbidden));
    assert!(!paths.db_path().exists(), "the refusal opened the store");

    let reader = Envelope::agent("r", Capabilities::parse("t", &["project.read"]));
    let mut home = home;
    let req = home.request();
    let mut ctx = Ctx::borrowed(&home.paths, &home.cfg, &mut home.conn);
    let page = authority::run(&mut ctx, &reader, req).unwrap();
    assert_eq!(page.events.len(), 1);
}

#[test]
fn a_payload_that_is_not_an_object_reads_as_empty() {
    let row = |payload: &str| ActivityRow {
        id: "e".into(),
        project_id: "p".into(),
        actor_principal_type: "user".into(),
        actor_principal_id: "u".into(),
        event_type: "project.created".into(),
        entity_type: "project".into(),
        entity_id: "p".into(),
        payload: payload.into(),
        created_at: 0,
    };
    for raw in ["[1]", "not json", "7"] {
        assert!(view(row(raw)).unwrap().payload.is_empty(), "{raw}");
    }
    let kept = view(row(r#"{"a":1}"#)).unwrap();
    assert_eq!(kept.payload["a"], json!(1));
    assert_eq!(kept.created_at, "1970-01-01T00:00:00.000Z");
    let e = view(ActivityRow {
        created_at: i64::MAX,
        ..row("{}")
    })
    .unwrap_err();
    assert_eq!(classify(&e).0, "internal_error");
}
