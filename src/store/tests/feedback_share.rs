#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The `feedback_events` reads sharing needs, against a real migrated
//! database written through the production insert.

use crate::store::feedback::{NewFeedbackEvent, insert_event};
use crate::store::{connection, feedback_share};
use rusqlite::Connection;

fn verdict<'a>(
    memory_id: &'a str,
    target_kind: &'a str,
    device: Option<&'a str>,
) -> NewFeedbackEvent<'a> {
    NewFeedbackEvent {
        query_id: "q-20260924-0badc0de",
        memory_id,
        verdict: "used",
        at: "2026-09-24T10:00:00Z",
        target_kind,
        provenance: "manual",
        surface: Some("cli"),
        actor: None,
        device,
        event_id: None,
    }
}

fn db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

#[test]
fn the_legacy_walk_skips_code_imported_and_stamped_rows_in_rowid_order() {
    let (_dir, conn) = db();
    let first = insert_event(&conn, &verdict("a1b2c3d4", "memory", None)).expect("first");
    insert_event(&conn, &verdict("42", "code", None)).expect("code");
    insert_event(
        &conn,
        &verdict(
            "a1b2c3d4",
            "memory",
            Some("feedfacefeedfacefeedfacefeedface"),
        ),
    )
    .expect("imported");
    let stamped = insert_event(&conn, &verdict("a1b2c3d4", "memory", None)).expect("stamped");
    feedback_share::stamp_event_id(&conn, stamped, "ev-0123456789abcdef0123456789abcdef")
        .expect("stamp");
    let last = insert_event(&conn, &verdict("e5f6a7b8", "memory", None)).expect("last");

    let walked: Vec<i64> = feedback_share::unshared_legacy_after(&conn, 0, 10)
        .expect("walk")
        .into_iter()
        .map(|v| v.id)
        .collect();
    assert_eq!(walked, vec![first, last]);
    let resumed: Vec<i64> = feedback_share::unshared_legacy_after(&conn, first, 10)
        .expect("resume")
        .into_iter()
        .map(|v| v.id)
        .collect();
    assert_eq!(resumed, vec![last], "the cursor resumes above the last row");
}

#[test]
fn a_reused_event_id_is_refused_by_the_unique_index() {
    let (_dir, conn) = db();
    let a = insert_event(&conn, &verdict("a1b2c3d4", "memory", None)).expect("a");
    let b = insert_event(&conn, &verdict("a1b2c3d4", "memory", None)).expect("b");
    let id = "ev-0123456789abcdef0123456789abcdef";
    feedback_share::stamp_event_id(&conn, a, id).expect("first stamp");
    assert!(
        feedback_share::stamp_event_id(&conn, b, id).is_err(),
        "one event id names one verdict"
    );
}

#[test]
fn the_event_ids_of_a_memory_are_its_journalled_memory_verdicts_only() {
    let (_dir, conn) = db();
    let shared = insert_event(&conn, &verdict("a1b2c3d4", "memory", None)).expect("shared");
    insert_event(&conn, &verdict("a1b2c3d4", "memory", None)).expect("local only");
    let code = insert_event(&conn, &verdict("a1b2c3d4", "code", None)).expect("code");
    feedback_share::stamp_event_id(&conn, shared, "ev-00000000000000000000000000000001")
        .expect("stamp");
    feedback_share::stamp_event_id(&conn, code, "ev-00000000000000000000000000000002")
        .expect("stamp code");
    assert_eq!(
        feedback_share::event_ids_for_memory(&conn, "a1b2c3d4").expect("ids"),
        vec!["ev-00000000000000000000000000000001".to_string()]
    );
}
