#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Coverage for `src/store/project_changes.rs` against a real migrated
//! `comemory.db` with foreign keys on: the feed pages by `seq`, a dropped
//! transaction leaves no row, and a deletion row — like every earlier row —
//! outlives the project whose cascade removes its activity trail; and the
//! rebuild preservation copy keeps every `seq`.

use comemory::store::connection::{self, write_transaction};
use comemory::store::project_activity::{self, NewProjectEvent};
use comemory::store::project_changes::{NewChange, append, bounds, page};
use comemory::store::projects::{NewProject, insert_project};
use comemory::store::rebuild_copy::copy_preserved_tables_from_old;
use rusqlite::Connection;
use tempfile::tempdir;

const P: &str = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2a";
const E: &str = "1f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2a";

fn change<'a>(event_id: &'a str, op: &'a str) -> NewChange<'a> {
    NewChange {
        project_id: P,
        event_id,
        entity_type: "project",
        op,
        at_ms: 7,
    }
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

/// A project, its `project.created` event and the feed row they share.
fn seed(conn: &Connection) {
    insert_project(
        conn,
        &NewProject {
            id: P,
            slug: "ship",
            key_prefix: "SHIP",
            name: "Ship",
            outcome: "Shipped",
            constraints: "[]",
            non_goals: "[]",
            lead_type: "user",
            lead_id: "local-operator",
            target_date: None,
            creator_type: "user",
            creator_id: "local-operator",
            at_ms: 7,
        },
    )
    .unwrap();
    project_activity::insert(
        conn,
        &NewProjectEvent {
            id: E,
            project_id: P,
            actor_type: "user",
            actor_id: "local-operator",
            event_type: "project.created",
            entity_type: "project",
            entity_id: P,
            payload: "{}",
            at_ms: 7,
        },
    )
    .unwrap();
    append(conn, &change(E, "changed")).unwrap();
}

#[test]
fn the_feed_pages_ascending_after_a_cursor_and_reports_its_bounds() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    assert_eq!(bounds(&conn).unwrap(), None);
    assert!(page(&conn, 0, 10).unwrap().is_empty());
    for event in ["e1", "e2", "e3"] {
        append(&conn, &change(event, "changed")).unwrap();
    }
    assert_eq!(bounds(&conn).unwrap(), Some((1, 3)));
    let rows = page(&conn, 1, 1).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].seq, rows[0].event_id.as_str()), (2, "e2"));
    let rest: Vec<i64> = page(&conn, 1, 10).unwrap().iter().map(|r| r.seq).collect();
    assert_eq!(rest, [2, 3]);
    assert!(page(&conn, 3, 10).unwrap().is_empty());
}

#[test]
fn a_dropped_transaction_leaves_no_row() {
    let dir = tempdir().unwrap();
    let mut conn = connection::open(dir.path().join("comemory.db")).unwrap();
    {
        let tx = write_transaction(&mut conn).unwrap();
        seed(&tx);
        assert_eq!(
            count(&tx, "project_changes"),
            1,
            "visible inside its transaction"
        );
    }
    assert_eq!(count(&conn, "project_changes"), 0);
    assert_eq!(count(&conn, "projects"), 0);
    assert_eq!(bounds(&conn).unwrap(), None);
}

#[test]
fn a_deletion_row_and_the_earlier_rows_outlive_the_project_cascade() {
    let dir = tempdir().unwrap();
    let mut conn = connection::open(dir.path().join("comemory.db")).unwrap();
    seed(&conn);
    let tx = write_transaction(&mut conn).unwrap();
    append(&tx, &change(P, "deleted")).unwrap();
    tx.execute("DELETE FROM projects WHERE id = ?1", [P])
        .unwrap();
    tx.commit().unwrap();

    assert_eq!(count(&conn, "projects"), 0);
    assert_eq!(
        count(&conn, "project_activity_events"),
        0,
        "the cascade removed the activity trail"
    );
    let rows = page(&conn, 0, 10).unwrap();
    let kept: Vec<(i64, &str, &str, &str)> = rows
        .iter()
        .map(|r| {
            (
                r.seq,
                r.project_id.as_str(),
                r.event_id.as_str(),
                r.op.as_str(),
            )
        })
        .collect();
    assert_eq!(kept, [(1, P, E, "changed"), (2, P, P, "deleted")]);
}

#[test]
fn the_project_change_feed_keeps_its_seq_across_the_preservation_copy() {
    let old_dir = tempdir().expect("old tempdir");
    let old_path = old_dir.path().join("comemory.db");
    {
        let old = connection::open(&old_path).expect("old db");
        for event in ["e1", "e2", "e3"] {
            append(&old, &change(event, "changed")).expect("append");
        }
    }

    let new_dir = tempdir().expect("new tempdir");
    let mut conn = connection::open(new_dir.path().join("comemory.db")).expect("new db");
    copy_preserved_tables_from_old(&mut conn, &old_path).expect("copy");

    let kept: Vec<(i64, String)> = page(&conn, 0, 10)
        .expect("page")
        .into_iter()
        .map(|r| (r.seq, r.event_id))
        .collect();
    assert_eq!(
        kept,
        [(1, "e1".into()), (2, "e2".into()), (3, "e3".into())],
        "every row keeps its seq"
    );
    let entity_type: String = conn
        .query_row(
            "SELECT entity_type FROM project_changes WHERE seq = 1",
            [],
            |r| r.get(0),
        )
        .expect("entity_type");
    assert_eq!(entity_type, "project");
    append(&conn, &change("e4", "changed")).expect("append after the copy");
    let next: Vec<i64> = page(&conn, 3, 10)
        .expect("page after the old head")
        .into_iter()
        .map(|r| r.seq)
        .collect();
    assert_eq!(next, [4], "the next mutation continues above the old head");
}
