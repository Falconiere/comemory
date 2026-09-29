#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Coverage for `src/store/project_activity.rs` against a real migrated
//! `comemory.db`: a keyset walk over `(created_at, id)` in either direction
//! returns one project's events exactly once at every page size, even when a
//! page boundary falls inside a run of events sharing one millisecond, and
//! the ascending walk is the descending one reversed.

use comemory::store::connection;
use comemory::store::project_activity::{self, ActivityPage, ActivityRow, NewProjectEvent};
use comemory::store::projects::{NewProject, insert_project};
use rusqlite::Connection;
use tempfile::tempdir;

const P: &str = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2a";
const OTHER: &str = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2b";
/// Each event's `created_at`: runs of ties, so page edges land inside them.
const STAMPS: [i64; 13] = [5, 5, 5, 7, 7, 9, 9, 9, 9, 11, 13, 13, 13];

fn project(conn: &Connection, id: &str, key: &str) {
    let slug = key.to_lowercase();
    insert_project(
        conn,
        &NewProject {
            id,
            slug: &slug,
            key_prefix: key,
            name: key,
            outcome: "Shipped",
            constraints: "[]",
            non_goals: "[]",
            lead_type: "user",
            lead_id: "local-operator",
            target_date: None,
            creator_type: "user",
            creator_id: "local-operator",
            at_ms: 1,
        },
    )
    .unwrap();
}

fn event(conn: &Connection, id: &str, project_id: &str, at_ms: i64) {
    project_activity::insert(
        conn,
        &NewProjectEvent {
            id,
            project_id,
            actor_type: "user",
            actor_id: "local-operator",
            event_type: "project.created",
            entity_type: "project",
            entity_id: project_id,
            payload: r#"{"n":1}"#,
            at_ms,
        },
    )
    .unwrap();
}

/// A store with [`STAMPS`] events on `P`, their ids inserted out of sort
/// order, plus events on another project at the same stamps; `P`'s ids.
fn seeded() -> (tempfile::TempDir, Connection, Vec<String>) {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    project(&conn, P, "SHIP");
    project(&conn, OTHER, "OTHER");
    let mut ids = Vec::new();
    for (n, at) in STAMPS.iter().enumerate() {
        // Reverse the id order against insertion so rowid order is no help.
        let id = format!("{:08x}-0000-4000-8000-000000000000", 0xff - n);
        event(&conn, &id, P, *at);
        event(
            &conn,
            &format!("{:08x}-0000-4000-8000-00000000000f", n),
            OTHER,
            *at,
        );
        ids.push(id);
    }
    (dir, conn, ids)
}

/// Walk `P` to the end with pages of `limit`, the next page starting after
/// the last row of the previous one.
fn walk(conn: &Connection, ascending: bool, limit: i64) -> Vec<ActivityRow> {
    let mut all: Vec<ActivityRow> = Vec::new();
    loop {
        let after = all.last().map(|row| (row.created_at, row.id.clone()));
        let page = project_activity::page(
            conn,
            &ActivityPage {
                project_id: P,
                ascending,
                after: after.as_ref().map(|(at, id)| (*at, id.as_str())),
                limit,
            },
        )
        .unwrap();
        let done = (page.len() as i64) < limit;
        all.extend(page);
        if done {
            return all;
        }
    }
}

#[test]
fn both_orders_walk_the_same_set_in_reverse_at_every_page_size() {
    let (_dir, conn, ids) = seeded();
    let mut expected: Vec<(i64, String)> = STAMPS.iter().copied().zip(ids).collect();
    expected.sort();
    let expected: Vec<String> = expected.into_iter().map(|(_, id)| id).collect();
    for limit in 1..=(STAMPS.len() as i64 + 1) {
        let asc: Vec<String> = walk(&conn, true, limit).into_iter().map(|r| r.id).collect();
        let mut desc: Vec<String> = walk(&conn, false, limit)
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(asc, expected, "asc walk at limit {limit}");
        desc.reverse();
        assert_eq!(desc, expected, "desc walk reversed at limit {limit}");
    }
}

#[test]
fn a_row_reads_back_column_for_column() {
    let (_dir, conn, ids) = seeded();
    let first = project_activity::page(
        &conn,
        &ActivityPage {
            project_id: P,
            ascending: true,
            after: None,
            limit: 1,
        },
    )
    .unwrap();
    let smallest = ids[..3].iter().min().unwrap();
    assert_eq!(
        first,
        vec![ActivityRow {
            id: smallest.clone(),
            project_id: P.to_string(),
            actor_principal_type: "user".to_string(),
            actor_principal_id: "local-operator".to_string(),
            event_type: "project.created".to_string(),
            entity_type: "project".to_string(),
            entity_id: P.to_string(),
            payload: r#"{"n":1}"#.to_string(),
            created_at: 5,
        }]
    );
}

#[test]
fn a_cursor_inside_a_tie_continues_from_the_next_id_in_either_direction() {
    let (_dir, conn, _ids) = seeded();
    // The middle of the four events at 9: ids sort within the tie.
    let asc = walk(&conn, true, 100);
    let at_nine: Vec<&ActivityRow> = asc.iter().filter(|r| r.created_at == 9).collect();
    let pivot = at_nine[1];
    let page = |ascending| {
        project_activity::page(
            &conn,
            &ActivityPage {
                project_id: P,
                ascending,
                after: Some((9, pivot.id.as_str())),
                limit: 2,
            },
        )
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect::<Vec<_>>()
    };
    assert_eq!(
        page(true),
        vec![at_nine[2].id.clone(), at_nine[3].id.clone()]
    );
    let before: Vec<String> = asc
        .iter()
        .filter(|r| r.created_at == 7)
        .map(|r| r.id.clone())
        .collect();
    assert_eq!(page(false), vec![at_nine[0].id.clone(), before[1].clone()]);
}
