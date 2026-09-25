#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Migration 0027 against a real database: the exchange-client tables exist
//! with their keys and checks, `replica_cursor` is keyed by
//! `(api_url, workspace_id)`, and the outbox carries its new columns — both on
//! a fresh database and on one a 0026-era binary created and then upgraded.

use comemory::store::connection;
use comemory::store::migrate::list::MIGRATIONS;
use rusqlite::Connection;

fn fresh() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    // `open` runs every migration.
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT name FROM pragma_table_info('{table}') ORDER BY cid"
        ))
        .expect("prepare");
    statement
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<Vec<String>, _>>()
        .expect("collect")
}

fn primary_key(conn: &Connection, table: &str) -> Vec<String> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT name FROM pragma_table_info('{table}') WHERE pk > 0 ORDER BY pk"
        ))
        .expect("prepare");
    statement
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<Vec<String>, _>>()
        .expect("collect")
}

#[test]
fn every_exchange_table_is_keyed_by_the_session_key() {
    let (_dir, conn) = fresh();
    assert_eq!(
        primary_key(&conn, "sync_exchange"),
        ["api_url", "workspace_id"]
    );
    assert_eq!(
        primary_key(&conn, "sync_policy_snapshot"),
        ["api_url", "workspace_id"]
    );
    assert_eq!(
        primary_key(&conn, "replica_cursor"),
        ["api_url", "workspace_id"]
    );
    assert_eq!(
        primary_key(&conn, "replica_pull_hold"),
        ["api_url", "workspace_id", "stream_epoch", "from_sequence"]
    );
    assert_eq!(
        primary_key(&conn, "replica_binding"),
        ["api_url", "workspace_id", "entity_kind", "entity_key"]
    );
    assert_eq!(
        primary_key(&conn, "replica_replay"),
        ["api_url", "workspace_id", "entity_kind", "entity_key"]
    );
}

#[test]
fn the_checks_refuse_a_state_the_client_never_writes() {
    let (_dir, conn) = fresh();
    let bad_network = conn.execute(
        "INSERT INTO sync_exchange (api_url, workspace_id, network_state, updated_at) \
         VALUES ('https://a', 'ws', 'sleeping', 'now')",
        [],
    );
    assert!(bad_network.is_err(), "an unknown network state is refused");
    let bad_protocol = conn.execute(
        "INSERT INTO sync_exchange (api_url, workspace_id, protocol, updated_at) \
         VALUES ('https://a', 'ws', 'replica-v2', 'now')",
        [],
    );
    assert!(bad_protocol.is_err(), "an unknown protocol is refused");
    let bad_reason = conn.execute(
        "INSERT INTO replica_pull_hold (api_url, workspace_id, stream_epoch, from_sequence, \
         to_sequence, reason, recorded_at) VALUES ('https://a', 'ws', 'e', 1, 1, 'bored', 'now')",
        [],
    );
    assert!(bad_reason.is_err(), "an unknown hold reason is refused");
    let fine = conn.execute(
        "INSERT INTO sync_exchange (api_url, workspace_id, updated_at) \
         VALUES ('https://a', 'ws', 'now')",
        [],
    );
    assert_eq!(fine.expect("defaults apply"), 1);
    let state: String = conn
        .query_row("SELECT network_state FROM sync_exchange", [], |r| r.get(0))
        .expect("read");
    assert_eq!(state, "ok", "a new key starts with the network open");
}

/// Build a genuine 0026-era database by replaying `0001..=0026` verbatim, with
/// their post-passes and markers, exactly as the previous release left one.
fn build_v26_db(path: &std::path::Path) -> Connection {
    // Opening a scratch database through the real connection registers
    // sqlite-vec for this process, which the raw replay's vec0 tables need.
    let scratch = path.with_file_name("scratch-vec-register.db");
    drop(connection::open(&scratch).expect("register sqlite-vec"));
    let mut conn = Connection::open(path).expect("open raw");
    comemory::store::tokenizer::ffi::register(&conn).expect("register identifier tokenizer");
    for m in MIGRATIONS.iter().take(26) {
        conn.execute_batch(m.sql)
            .unwrap_or_else(|e| panic!("replay {}: {e}", m.key));
        if let Some(post) = m.post {
            post(&mut conn).unwrap_or_else(|e| panic!("post {}: {e}", m.key));
        }
        for marker in m.markers {
            conn.execute(
                "INSERT OR IGNORE INTO schema_meta(key, value) VALUES (?1, '1')",
                [marker],
            )
            .expect("marker");
        }
    }
    conn.execute(
        "INSERT OR REPLACE INTO schema_meta(key, value) VALUES ('version', '26')",
        [],
    )
    .expect("version");
    conn
}

#[test]
fn an_upgraded_0026_database_gains_the_outbox_columns_and_keeps_its_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = build_v26_db(&path);
    assert_eq!(MIGRATIONS[25].key, "0026_replica_events");
    conn.execute(
        "INSERT INTO replica_operation (operation_id, entity_kind, entity_key, op, \
         schema_version, state, created_at, updated_at) \
         VALUES ('op-20260924-aaaaaaaa', 'memory', 'a1b2c3d4', 'upsert', 1, 'pending', 't', 't')",
        [],
    )
    .expect("a 0026 outbox row");
    drop(conn);

    // The real open path: preflight, snapshot, then every missing migration.
    let conn = connection::open(&path).expect("upgrade to head");

    for column in [
        "hold_reason",
        "hold_detail",
        "api_url",
        "workspace_id",
        "wire_repository",
        "upstream_epoch",
    ] {
        assert!(
            columns(&conn, "replica_operation").contains(&column.to_string()),
            "replica_operation gained {column}"
        );
    }
    let kept: (String, Option<String>) = conn
        .query_row(
            "SELECT state, hold_reason FROM replica_operation \
             WHERE operation_id = 'op-20260924-aaaaaaaa'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the row survived");
    assert_eq!(kept, ("pending".to_string(), None), "still owed, not held");
    assert_eq!(
        primary_key(&conn, "replica_cursor"),
        ["api_url", "workspace_id"]
    );
    assert!(columns(&conn, "replica_cursor").contains(&"anchor_operation_id".to_string()));
}
