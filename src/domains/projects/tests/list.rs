#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/list.rs`: pages walk to the end
//! with `nextCursor` even when every row shares one millisecond, the page
//! size and the filters refuse bad values with the right edge, and archived
//! projects stay out unless asked for.

use comemory::config::{Config, Paths};
use comemory::domains::projects::authority::{self, Envelope};
use comemory::domains::projects::create;
use comemory::domains::projects::list::{self, Request};
use comemory::store::{Connection, connection};
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};

/// A data directory holding `n` created projects, and their ids.
struct Seeded {
    _dir: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    conn: Connection,
    ids: Vec<String>,
}

impl Seeded {
    fn new(n: usize) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure_dirs().unwrap();
        let cfg = Config::defaults();
        let mut conn = connection::open(paths.db_path()).unwrap();
        let mut ids = Vec::new();
        {
            let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
            for k in 0..n {
                let req: create::Request = serde_json::from_value(serde_json::json!({
                    "idempotencyKey": format!("k{k}"), "name": format!("P{k}"), "keyPrefix": format!("P{k}"),
                    "outcome": "o"
                }))
                .unwrap();
                let created = authority::run(&mut ctx, &Envelope::local_operator(), req).unwrap();
                ids.push(created.project.id);
            }
        }
        Self {
            _dir: dir,
            paths,
            cfg,
            conn,
            ids,
        }
    }

    fn list(&mut self, req: Request) -> comemory::errors::Result<list::Response> {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        authority::run(&mut ctx, &Envelope::local_operator(), req)
    }
}

#[test]
fn a_walk_returns_every_project_once_even_within_one_millisecond() {
    let mut home = Seeded::new(7);
    // Every row in one millisecond: the id alone orders the walk.
    home.conn
        .execute_batch("UPDATE projects SET created_at = 1000")
        .unwrap();
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = home
            .list(Request {
                limit: Some(3),
                cursor,
                ..Request::default()
            })
            .unwrap();
        seen.extend(page.projects.into_iter().map(|p| p.id));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let mut expected = home.ids.clone();
    expected.sort();
    expected.reverse();
    assert_eq!(seen, expected);
    let first = home.list(Request::default()).unwrap();
    assert_eq!((first.projects.len(), first.next_cursor), (7, None));
}

#[test]
fn refusals_use_their_edge() {
    let mut home = Seeded::new(1);
    let refusals = [
        (
            Request {
                limit: Some(101),
                ..Request::default()
            },
            Class::Unprocessable,
        ),
        (
            Request {
                limit: Some(0),
                ..Request::default()
            },
            Class::Unprocessable,
        ),
        (
            Request {
                cursor: Some("abc".into()),
                ..Request::default()
            },
            Class::BadRequest,
        ),
        (
            Request {
                status: Some("sleeping".into()),
                ..Request::default()
            },
            Class::BadRequest,
        ),
        (
            Request {
                health: Some("great".into()),
                ..Request::default()
            },
            Class::BadRequest,
        ),
    ];
    for (req, class) in refusals {
        let e = home.list(req).unwrap_err();
        assert_eq!(classify(&e), ("invalid_request", class));
    }
}

#[test]
fn archived_projects_stay_out_unless_asked_and_filters_narrow() {
    let mut home = Seeded::new(3);
    let archived = home.ids[0].clone();
    home.conn
        .execute_batch(&format!(
            "UPDATE projects SET archived_at = 5 WHERE id = '{archived}'"
        ))
        .unwrap();
    assert_eq!(home.list(Request::default()).unwrap().projects.len(), 2);
    let all = home
        .list(Request {
            include_archived: Some(true),
            ..Request::default()
        })
        .unwrap();
    assert_eq!(all.projects.len(), 3);
    let drafts = home
        .list(Request {
            status: Some("draft".into()),
            ..Request::default()
        })
        .unwrap();
    assert_eq!(drafts.projects.len(), 2);
    let active = home
        .list(Request {
            status: Some("active".into()),
            ..Request::default()
        })
        .unwrap();
    assert!(active.projects.is_empty());
}

#[test]
fn a_cursor_walk_keeps_its_filters_across_rows_sharing_one_millisecond() {
    let mut home = Seeded::new(6);
    home.conn
        .execute_batch("UPDATE projects SET created_at = 1000")
        .unwrap();
    let archived = [home.ids[1].clone(), home.ids[4].clone()];
    for id in &archived {
        home.conn
            .execute_batch(&format!(
                "UPDATE projects SET archived_at = 5 WHERE id = '{id}'"
            ))
            .unwrap();
    }
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let req = Request {
            limit: Some(1),
            cursor,
            status: Some("draft".into()),
            ..Request::default()
        };
        let page = home.list(req).unwrap();
        seen.extend(page.projects.into_iter().map(|p| p.id));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let mut expected: Vec<String> = home
        .ids
        .iter()
        .filter(|id| !archived.contains(id))
        .cloned()
        .collect();
    expected.sort();
    expected.reverse();
    assert_eq!(seen, expected);
}
