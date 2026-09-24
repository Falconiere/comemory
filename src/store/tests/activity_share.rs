#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The capture batch over a real migrated database written through the
//! production insert: local runs only, in id order, resumable.

use crate::store::activity::{NewActivityRow, insert};
use crate::store::{activity_share, connection};

fn run<'a>(command: &'a str, device: Option<&'a str>) -> NewActivityRow<'a> {
    NewActivityRow {
        at: "2026-09-24T10:00:00Z",
        command,
        source: "cli",
        actor: None,
        repo: Some("demo"),
        duration_ms: 3,
        ok: true,
        error_code: None,
        summary: None,
        device,
        event_id: None,
    }
}

#[test]
fn the_batch_holds_local_runs_only_and_resumes_above_its_cursor() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let first = insert(&conn, &run("save", None)).expect("first");
    insert(
        &conn,
        &run("find", Some("feedfacefeedfacefeedfacefeedface")),
    )
    .expect("imported");
    let last = insert(&conn, &run("find", None)).expect("last");

    let batch: Vec<i64> = activity_share::capture_batch_after(&conn, 0, 10)
        .expect("batch")
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(
        batch,
        vec![first, last],
        "an imported run is never re-captured"
    );
    let resumed: Vec<i64> = activity_share::capture_batch_after(&conn, first, 1)
        .expect("resume")
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(resumed, vec![last]);

    activity_share::stamp_event_id(&conn, first, "ev-0123456789abcdef0123456789abcdef")
        .expect("stamp");
    assert!(
        activity_share::stamp_event_id(&conn, last, "ev-0123456789abcdef0123456789abcdef").is_err(),
        "one event id names one run"
    );
}
