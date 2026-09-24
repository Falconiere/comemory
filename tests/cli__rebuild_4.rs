#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory rebuild` — part 4: the state shared events depend on (#254).
//!
//! `activity_log` sat in `COPIED_TABLES` with no copy pass, so every rebuild
//! dropped the activity history while the coverage test, which only checks
//! the listing, stayed green. These cases drive the real binary over real
//! recorded runs and prove the feed, the device id, and the event identity
//! columns a shared event is deduplicated by all survive.

use assert_cmd::Command;
use rusqlite::Connection;
use tempfile::tempdir;

fn run(home: &std::path::Path, args: &[&str]) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .args(args)
        .assert()
        .success();
}

fn db(home: &std::path::Path) -> Connection {
    Connection::open(home.join("comemory.db")).expect("open db")
}

type ActivityRow = (
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn activity(home: &std::path::Path) -> Vec<ActivityRow> {
    let conn = db(home);
    let mut statement = conn
        .prepare("SELECT id, at, command, summary, event_id, device FROM activity_log ORDER BY id")
        .expect("prepare");
    statement
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

#[test]
fn rebuild_keeps_the_activity_feed_the_device_and_the_event_identity() {
    let home = tempdir().expect("home");
    let dir = home.path();
    run(
        dir,
        &[
            "save",
            "Rebuild replays memories from markdown and copies what markdown cannot hold.",
            "--kind",
            "decision",
        ],
    );
    run(dir, &["search", "rebuild replays memories"]);
    run(dir, &["find", "markdown cannot hold"]);
    // The identity columns a shared event carries, written onto real rows the
    // way the capture writes them.
    let conn = db(dir);
    conn.execute_batch(
        "UPDATE activity_log SET event_id = 'ev-' || lower(hex(randomblob(16))) \
          WHERE command = 'find'; \
         UPDATE activity_log SET device = 'feedfacefeedfacefeedfacefeedface' \
          WHERE command = 'search';",
    )
    .expect("stamp identity");
    let device: String = conn
        .query_row("SELECT device_id FROM replica_device", [], |r| r.get(0))
        .expect("device");
    drop(conn);
    let before = activity(dir);
    assert!(
        before.len() >= 3,
        "save, search and find were recorded: {before:?}"
    );

    run(dir, &["rebuild"]);

    assert_eq!(
        activity(dir),
        before,
        "the feed survives, ids and identity intact"
    );
    let after: String = db(dir)
        .query_row("SELECT device_id FROM replica_device", [], |r| r.get(0))
        .expect("device");
    assert_eq!(after, device, "the machine keeps its device id");
}

#[test]
fn rebuild_keeps_verdict_origin_and_payload_redaction() {
    let home = tempdir().expect("home");
    let dir = home.path();
    run(
        dir,
        &[
            "save",
            "A verdict's origin columns and a payload's redaction kind outlive a rebuild.",
            "--kind",
            "note",
        ],
    );
    let conn = db(dir);
    let (digest, id): (String, String) = conn
        .query_row(
            "SELECT payload_digest, entity_key FROM replica_feed WHERE entity_kind = 'memory'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("journalled memory");
    conn.execute_batch(&format!(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind, provenance, \
                                     event_id, device, surface, actor) \
         VALUES ('q-20260924-0badc0de', '{id}', 'used', '2026-09-24T10:00:00Z', 'memory', \
                 'manual', 'ev-0123456789abcdef0123456789abcdef', NULL, 'mcp', 'host/1.0'); \
         UPDATE replica_payload SET bytes = NULL, redacted_at = '2026-09-24T10:00:00Z', \
                redaction = 'expired' WHERE digest = '{digest}';"
    ))
    .expect("stamp origin and redaction");
    drop(conn);

    run(dir, &["rebuild"]);

    let conn = db(dir);
    let row: (String, Option<String>, String, String) = conn
        .query_row(
            "SELECT event_id, device, surface, actor FROM feedback_events",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("verdict survived");
    assert_eq!(
        row,
        (
            "ev-0123456789abcdef0123456789abcdef".to_string(),
            None,
            "mcp".to_string(),
            "host/1.0".to_string()
        )
    );
    let redaction: Option<String> = conn
        .query_row(
            "SELECT redaction FROM replica_payload WHERE digest = ?1",
            [&digest],
            |r| r.get(0),
        )
        .expect("payload survived");
    assert_eq!(redaction.as_deref(), Some("expired"));
}

#[test]
fn rebuild_keeps_a_fresh_identity_when_the_source_has_none_to_give() {
    let home = tempdir().expect("home");
    let dir = home.path();
    run(
        dir,
        &[
            "save",
            "An identity row the source lost is re-minted, never left empty.",
            "--kind",
            "note",
        ],
    );
    // The source table exists but holds no row — replacing the fresh row with
    // nothing would leave the rebuilt database with no device identity.
    db(dir)
        .execute("DELETE FROM replica_device", [])
        .expect("empty the source identity");

    run(dir, &["rebuild"]);

    let device: String = db(dir)
        .query_row("SELECT device_id FROM replica_device", [], |r| r.get(0))
        .expect("the rebuilt database has a device id");
    assert_eq!(device.len(), 32);
}

#[test]
fn rebuild_from_a_source_that_predates_v26_carries_the_new_columns_as_null() {
    let home = tempdir().expect("home");
    let dir = home.path();
    run(
        dir,
        &[
            "save",
            "A pre-v26 source copies by position, the new columns as NULL.",
            "--kind",
            "note",
        ],
    );
    run(dir, &["find", "pre-v26 source copies"]);
    let conn = db(dir);
    let id: String = conn
        .query_row("SELECT id FROM memories", [], |r| r.get(0))
        .expect("memory");
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind, provenance) \
         VALUES ('q-20260924-0badc0de', ?1, 'used', '2026-09-24T10:00:00Z', 'memory', 'manual')",
        [&id],
    )
    .expect("verdict");
    // The source as a v25 database had it: none of the v26 columns.
    conn.execute_batch(
        "DROP INDEX uq_feedback_events_event_id; DROP INDEX uq_activity_log_event_id; \
         DROP INDEX idx_replica_feed_kind_at; \
         ALTER TABLE feedback_events DROP COLUMN event_id; \
         ALTER TABLE feedback_events DROP COLUMN device; \
         ALTER TABLE feedback_events DROP COLUMN surface; \
         ALTER TABLE feedback_events DROP COLUMN actor; \
         ALTER TABLE activity_log DROP COLUMN event_id; \
         ALTER TABLE activity_log DROP COLUMN device; \
         ALTER TABLE replica_payload DROP COLUMN redaction;",
    )
    .expect("the v25 shape");
    let runs: i64 = conn
        .query_row("SELECT count(*) FROM activity_log", [], |r| r.get(0))
        .expect("runs");
    drop(conn);

    run(dir, &["rebuild"]);

    let conn = db(dir);
    let verdict: (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT provenance, event_id, surface FROM feedback_events",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("the verdict survived");
    assert_eq!(verdict, ("manual".to_string(), None, None));
    let copied: (i64, i64) = conn
        .query_row(
            "SELECT count(*), count(event_id) FROM activity_log",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("runs survived");
    assert_eq!(copied, (runs, 0), "every run, each with a NULL event id");
}
