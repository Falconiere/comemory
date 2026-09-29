#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/list.rs`: pages walk to the end
//! with `nextCursor` even when every row shares one millisecond, the page
//! size and the filters refuse bad values with the right edge, and archived
//! projects stay out unless asked for.

use comemory::config::{Config, Paths};
use comemory::domains::projects::create;
use comemory::domains::projects::list::{self, Request};
use comemory::domains::projects::principal::Principal;
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};

#[test]
fn a_walk_returns_every_project_once_and_refusals_use_their_edge() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let mut created = Vec::new();
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        for n in 0..7 {
            let req: create::Request = serde_json::from_value(serde_json::json!({
                "name": format!("P{n}"), "keyPrefix": format!("P{n}"), "outcome": "o"
            }))
            .unwrap();
            created.push(
                create::run(&mut ctx, &Principal::local_operator(), req)
                    .unwrap()
                    .project
                    .id,
            );
        }
    }
    // Every row in one millisecond: the id alone orders the walk.
    conn.execute_batch("UPDATE projects SET created_at = 1000")
        .unwrap();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = list::run(
            &mut ctx,
            Request {
                limit: Some(3),
                cursor,
                ..Request::default()
            },
        )
        .unwrap();
        seen.extend(page.projects.into_iter().map(|p| p.id));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let mut expected = created.clone();
    expected.sort();
    expected.reverse();
    assert_eq!(seen, expected);

    let first = list::run(&mut ctx, Request::default()).unwrap();
    assert_eq!((first.projects.len(), first.next_cursor), (7, None));

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
        let e = list::run(&mut ctx, req).unwrap_err();
        assert_eq!(classify(&e), ("invalid_request", class));
    }

    drop(ctx);
    conn.execute_batch(&format!(
        "UPDATE projects SET archived_at = 5 WHERE id = '{}'",
        created[0]
    ))
    .unwrap();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let live = list::run(&mut ctx, Request::default()).unwrap();
    assert_eq!(live.projects.len(), 6);
    let all = list::run(
        &mut ctx,
        Request {
            include_archived: Some(true),
            ..Request::default()
        },
    )
    .unwrap();
    assert_eq!(all.projects.len(), 7);
    let drafts = list::run(
        &mut ctx,
        Request {
            status: Some("draft".into()),
            ..Request::default()
        },
    )
    .unwrap();
    assert_eq!(drafts.projects.len(), 6);
}
