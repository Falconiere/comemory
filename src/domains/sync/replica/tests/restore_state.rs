#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/sync/replica/restore_state.rs` (#256, B-4):
//! the state and the pending record over a real migrated database, and every
//! replica and legacy sync route of the production router answering `503
//! restore_unverified` while either is present.

use comemory::config::Paths;
use comemory::domains::sync::replica::restore_state::{self, KEY, State};
use comemory::prelude::Error;
use comemory::store::connection;
use serde_json::json;

use crate::domains::sync::replica::test_support::{Home, tombstone};
use crate::test_common::serve_state;

#[test]
fn the_state_is_a_replica_key_set_read_and_cleared() {
    let home = Home::new();
    assert!(KEY.starts_with("replica_"), "a rebuild must carry it");
    assert_eq!(restore_state::read(&home.conn).expect("read"), None);
    restore_state::refuse_unverified(&home.paths, &home.conn).expect("nothing to refuse");

    restore_state::set(&home.conn, State::ErasureUnknown).expect("set");
    assert_eq!(
        restore_state::read(&home.conn).expect("read").as_deref(),
        Some("erasure_unknown")
    );
    let refused = restore_state::refuse_unverified(&home.paths, &home.conn);
    let Err(Error::RestoreUnverified(why)) = refused else {
        panic!("a set state refuses: {refused:?}");
    };
    assert!(why.contains("merge-erasures"), "{why}");

    restore_state::clear(&home.conn).expect("clear");
    assert_eq!(restore_state::read(&home.conn).expect("read"), None);
    restore_state::refuse_unverified(&home.paths, &home.conn).expect("allowed again");
}

#[test]
fn a_pending_restore_or_an_unknown_state_refuses() {
    let home = Home::new();
    std::fs::write(restore_state::pending_path(&home.paths), "{}").expect("pending");
    let Err(Error::RestoreUnverified(why)) =
        restore_state::refuse_unverified(&home.paths, &home.conn)
    else {
        panic!("a pending restore refuses");
    };
    assert!(why.contains("backup restore"), "{why}");
    std::fs::remove_file(restore_state::pending_path(&home.paths)).expect("remove");

    home.conn
        .execute(
            "INSERT INTO schema_meta(key, value) VALUES (?1, 'from_a_newer_build')",
            [KEY],
        )
        .expect("an unknown state");
    assert!(
        matches!(
            restore_state::refuse_unverified(&home.paths, &home.conn),
            Err(Error::RestoreUnverified(_))
        ),
        "fail closed on a state this build does not know"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_sync_route_answers_restore_unverified() {
    let session = serve_state::session(false);
    let paths = Paths::new(session.home.path());
    let first = serve_state::send(&session, "GET", "/api/v1/sync/replica/manifest", None).await;
    assert_eq!(first.status, 200, "{}", first.text);
    restore_state::set(
        &connection::open(paths.db_path()).expect("open"),
        State::ErasureUnknown,
    )
    .expect("set");
    let operation =
        serde_json::to_value(tombstone("op-20260925-dddddddd", "a1b2c3d4")).expect("op");
    let routes = [
        ("GET", "/api/v1/sync/replica/changes?since=0", None),
        ("GET", "/api/v1/sync/replica/manifest", None),
        ("GET", "/api/v1/sync/replica/events?since=0", None),
        (
            "POST",
            "/api/v1/sync/replica/import",
            Some(json!({"protocol": "replica-v1", "operations": []})),
        ),
        (
            "POST",
            "/api/v1/sync/replica/stage",
            Some(
                json!({"protocol": "replica-v1", "staging_id": "s-1", "part_index": 0,
                        "part_count": 2, "bytes": "{"}),
            ),
        ),
        (
            "POST",
            "/api/v1/sync/replica/activate",
            Some(json!({"protocol": "replica-v1", "staging_id": "s-2", "operation": operation})),
        ),
        ("GET", "/api/v1/sync/changes?since=0", None),
        ("GET", "/api/v1/sync/manifest", None),
        (
            "POST",
            "/api/v1/sync/import",
            Some(json!({"cursor": 0, "entries": []})),
        ),
        (
            "GET",
            "/api/v1/sync/code/manifest?repo=falconiere/comemory",
            None,
        ),
        (
            "POST",
            "/api/v1/sync/code/import",
            Some(json!({"repo": "falconiere/comemory"})),
        ),
    ];
    for (method, path, body) in routes.clone() {
        let resp = serve_state::send(&session, method, path, body).await;
        assert_eq!(resp.status, 503, "{method} {path}: {}", resp.text);
        assert_eq!(
            resp.json["error"]["code"], "restore_unverified",
            "{method} {path}: {}",
            resp.text
        );
    }

    restore_state::clear(&connection::open(paths.db_path()).expect("open")).expect("clear");
    std::fs::write(restore_state::pending_path(&paths), "{}").expect("pending");
    let pending = serve_state::send(&session, "GET", "/api/v1/sync/manifest", None).await;
    assert_eq!(pending.status, 503, "a pending swap refuses too");
    std::fs::remove_file(restore_state::pending_path(&paths)).expect("remove");
    let open = serve_state::send(&session, "GET", "/api/v1/sync/replica/manifest", None).await;
    assert_eq!(open.status, 200, "{}", open.text);
}
