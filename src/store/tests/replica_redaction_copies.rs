#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::replica_redaction_copies`] against a real
//! migrated database: a killed replay's scratch copy of an expired digest is
//! the one thing this reaches, and nothing else.

use comemory::store::connection;
use comemory::store::replica_redaction_copies::clear_replay_of;
use comemory::store::replica_replay::{self, ReplayRow};
use comemory::store::sync_exchange::ExchangeKey;

fn migrated_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn row(entity_key: &str, sequence: i64, digest: Option<&str>) -> ReplayRow {
    let digest_field = digest.map_or_else(String::new, |d| format!(r#","payload_digest":"{d}""#));
    ReplayRow {
        entity_kind: "memory".to_string(),
        entity_key: entity_key.to_string(),
        sequence,
        entry_json: format!(r#"{{"sequence":{sequence}{digest_field}}}"#),
    }
}

#[test]
fn a_scratch_row_naming_an_expired_digest_is_cleared() {
    let (_dir, conn) = migrated_db();
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    replica_replay::offer(&conn, &key, &row("m1", 10, Some("digest-expired"))).expect("offer m1");
    replica_replay::offer(&conn, &key, &row("m2", 11, Some("digest-live"))).expect("offer m2");

    let cleared = clear_replay_of(&conn, &["digest-expired".to_string()]).expect("clear");

    assert_eq!(cleared, 1);
    let left = replica_replay::next(&conn, &key, 10).expect("next");
    assert_eq!(left, vec![row("m2", 11, Some("digest-live"))]);
}

#[test]
fn an_empty_digest_list_is_a_no_op() {
    let (_dir, conn) = migrated_db();
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    replica_replay::offer(&conn, &key, &row("m1", 10, Some("digest-a"))).expect("offer");

    assert_eq!(clear_replay_of(&conn, &[]).expect("clear"), 0);
    assert_eq!(
        replica_replay::next(&conn, &key, 10).expect("next").len(),
        1
    );
}

#[test]
fn a_row_whose_entry_names_no_digest_is_left_alone() {
    let (_dir, conn) = migrated_db();
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    // A tombstone entry carries no `payload_digest` at all.
    replica_replay::offer(&conn, &key, &row("m1", 10, None)).expect("offer");

    let cleared = clear_replay_of(&conn, &["anything".to_string()]).expect("clear");

    assert_eq!(cleared, 0, "a digest-less entry never matches");
    assert_eq!(
        replica_replay::next(&conn, &key, 10).expect("next").len(),
        1
    );
}
