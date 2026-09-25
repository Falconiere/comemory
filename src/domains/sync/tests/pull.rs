#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! `run_pull` against a loopback platform that returns a real upsert entry.

use time::OffsetDateTime;

use comemory::config::{Config, Paths};
use comemory::domains::memories::id::memory_id;
use comemory::domains::memories::{Kind, MemoryStore};
use comemory::domains::sync::AuthFile;
use comemory::domains::sync::drain::{
    self,
    session::{Legs, Mode},
};
use comemory::domains::sync::pull;
use comemory::store::connection;
use comemory::store::sync_state;
use comemory::utilities::digest::sha256_hex;

use crate::test_common as common;
use crate::test_common::sync_platform_server::{SyncPlatformServer, SyncPlatformState};

fn upsert_wire_entry(body: &str, seq: i64) -> serde_json::Value {
    let id = memory_id(body);
    let content_hash = sha256_hex(body.trim_end().as_bytes());
    let created = OffsetDateTime::now_utc();
    serde_json::json!({
        "seq": seq,
        "op": "upsert",
        "id": id,
        "content_hash": content_hash,
        "at": "2026-09-06T12:00:00Z",
        "author": "peer-device",
        "record": {
            "frontmatter": {
                "id": id,
                "kind": "note",
                "repo": "codasignal/foo",
                "tags": ["sync"],
                "created": created.format(&time::format_description::well_known::Iso8601::DEFAULT)
                    .expect("iso"),
                "quality": 3,
                "schema": 1,
                "content_hash": content_hash,
                "references": {"symbols": [], "files": []},
                "relations": {
                    "supersedes": [],
                    "conflicts_with": [],
                    "derived_from": []
                }
            },
            "body": body,
            "vector": null
        }
    })
}

fn seed_auth(paths: &Paths, api_url: &str, secret: &str, workspace: &str) {
    common::auth_fixture::seed_org_auth(paths, api_url, secret, workspace);
}

/// Drain the pull direction the way `comemory sync --action pull` does.
fn pull_all(
    paths: &Paths,
    cfg: &Config,
    conn: &mut comemory::store::Connection,
    auth: &AuthFile,
) -> pull::PullStats {
    let drained = drain::drain(paths, cfg, conn, auth, (Mode::Manual, Legs::Pull)).expect("drain");
    assert_eq!(drained.error, None, "the pull reached the platform");
    drained
        .legacy
        .and_then(|l| l.pull)
        .expect("a legacy pull leg")
}

#[test]
fn pull_applies_remote_upsert_and_advances_cursor() {
    let body = "remote upsert pulled into a fresh local store";
    let platform = SyncPlatformState {
        head_seq: 7,
        changes: serde_json::json!([upsert_wire_entry(body, 7)]),
        ..Default::default()
    };
    let server = SyncPlatformServer::start(platform);
    let secret = server.snapshot().secret;

    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let cfg = Config::defaults();
    let workspace = "ws-org";
    seed_auth(&paths, &server.base, &secret, workspace);

    let auth = AuthFile::load(&paths).expect("load").expect("auth");
    let pull_stats = pull_all(&paths, &cfg, &mut conn, &auth);
    assert_eq!(pull_stats.pulled, 1);
    assert_eq!(pull_stats.last_pulled_seq, 7);

    let row = sync_state::get(&conn, workspace)
        .expect("get")
        .expect("row");
    assert_eq!(row.pulled_seq, 7);
    assert!(row.last_sync_at.is_some());

    let store = MemoryStore::new(paths);
    let rec = store.load(&memory_id(body)).expect("memory");
    assert_eq!(rec.body, body);
    assert_eq!(rec.frontmatter.kind, Kind::Note);
}

#[test]
fn empty_remote_changes_without_a_continuation_do_not_infer_progress() {
    let platform = SyncPlatformState {
        head_seq: 42,
        changes: serde_json::json!([]),
        ..Default::default()
    };
    let server = SyncPlatformServer::start(platform);
    let secret = server.snapshot().secret;

    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let cfg = Config::defaults();
    let workspace = "ws-org";
    seed_auth(&paths, &server.base, &secret, workspace);

    let auth = AuthFile::load(&paths).expect("load").expect("auth");
    let pull_stats = pull_all(&paths, &cfg, &mut conn, &auth);
    assert_eq!(pull_stats.pulled, 0);
    assert_eq!(pull_stats.last_pulled_seq, 0);
}

/// A fresh store logged in to `server`, and the drain's legacy pull over it.
fn legacy_pull(
    server: &SyncPlatformServer,
) -> (
    tempfile::TempDir,
    Paths,
    comemory::store::Connection,
    pull::PullStats,
) {
    let secret = server.snapshot().secret;
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    seed_auth(&paths, &server.base, &secret, "ws-org");
    let auth = AuthFile::load(&paths).expect("load").expect("auth");
    let stats = pull_all(&paths, &Config::defaults(), &mut conn, &auth);
    (home, paths, conn, stats)
}

/// A tombstone whose id is not a memory id: the import answers `invalid`.
fn invalid_tombstone(seq: i64) -> serde_json::Value {
    serde_json::json!({
        "seq": seq,
        "op": "tombstone",
        "id": "not-a-memory-id",
        "content_hash": "0".repeat(64),
        "at": "2026-09-06T12:00:00Z",
        "author": "peer-device",
        "record": null
    })
}

#[test]
fn legacy_stall_stops_before_an_entry_the_import_calls_invalid() {
    let before = "a peer memory the legacy pull applies before the invalid entry";
    let after = "a peer memory past the invalid entry, never passed by the cursor";
    let platform = SyncPlatformState {
        head_seq: 9,
        changes: serde_json::json!([
            upsert_wire_entry(before, 7),
            invalid_tombstone(8),
            upsert_wire_entry(after, 9),
        ]),
        ..Default::default()
    };
    let server = SyncPlatformServer::start(platform);

    let (_home, paths, conn, stats) = legacy_pull(&server);

    assert_eq!(
        stats.stalled.as_ref().map(|(seq, _)| *seq),
        Some(8),
        "{stats:?}"
    );
    assert_eq!(
        stats.last_pulled_seq, 7,
        "the cursor names the entry before"
    );
    let row = sync_state::get(&conn, "ws-org").expect("get").expect("row");
    assert_eq!(
        row.pulled_seq, 7,
        "pulled_seq never passes the invalid entry"
    );
    let key = comemory::store::sync_exchange::ExchangeKey::new(&server.base, "ws-org");
    let exchange = comemory::store::sync_exchange::load(&conn, &key)
        .expect("load")
        .expect("row");
    assert_eq!(exchange.stall_sequence, Some(8), "status names the entry");
    assert!(
        MemoryStore::new(paths).load(&memory_id(before)).is_ok(),
        "the entry before it applied"
    );
}

#[test]
fn legacy_hold_passes_a_secret_entry_and_keeps_the_rest_flowing() {
    let secret_body = "Rotated the deploy credential: api_key=Zx9Qp2Lm7Rt4Wn8Yc3Vb6Hj1Ks5Fd0Ae (rotate before release).";
    let fine = "a peer memory that follows the refused one and still applies";
    let platform = SyncPlatformState {
        head_seq: 8,
        changes: serde_json::json!([
            upsert_wire_entry(secret_body, 7),
            upsert_wire_entry(fine, 8)
        ]),
        ..Default::default()
    };
    let server = SyncPlatformServer::start(platform);

    let (_home, paths, conn, stats) = legacy_pull(&server);

    assert_eq!((stats.pulled, stats.held), (1, 1), "{stats:?}");
    assert!(
        stats.stalled.is_none(),
        "a refusal with no local remedy never stalls"
    );
    assert_eq!(
        sync_state::get(&conn, "ws-org")
            .expect("get")
            .expect("row")
            .pulled_seq,
        8
    );
    let key = comemory::store::sync_exchange::ExchangeKey::new(&server.base, "ws-org");
    let holds = comemory::store::replica_pull_hold::list(
        &conn,
        &key,
        comemory::store::replica_pull_hold::LEGACY_EPOCH,
    )
    .expect("holds");
    assert_eq!(holds.len(), 1, "{holds:?}");
    assert_eq!(
        (holds[0].from_sequence, holds[0].reason.as_str()),
        (7, "secret")
    );
    let store = MemoryStore::new(paths);
    assert!(
        store.load(&memory_id(secret_body)).is_err(),
        "the secret never landed"
    );
    assert!(
        store.load(&memory_id(fine)).is_ok(),
        "the next entry applied"
    );
}

#[test]
fn legacy_stall_on_a_write_failure_names_the_entry_that_failed() {
    let first = "a peer memory the legacy pull applies before the blocked one";
    let blocked = "a peer memory whose markdown path is taken by a directory";
    let last = "a peer memory past the blocked one, never passed by the cursor";
    let platform = SyncPlatformState {
        head_seq: 9,
        changes: serde_json::json!([
            upsert_wire_entry(first, 7),
            upsert_wire_entry(blocked, 8),
            upsert_wire_entry(last, 9),
        ]),
        ..Default::default()
    };
    let server = SyncPlatformServer::start(platform);
    let secret = server.snapshot().secret;
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    seed_auth(&paths, &server.base, &secret, "ws-org");
    // The write target of the middle entry is a directory: its import fails
    // hard (I/O), after the first entry already committed.
    let store = MemoryStore::new(paths.clone());
    std::fs::create_dir_all(store.planned_path(blocked)).expect("block the write");
    let auth = AuthFile::load(&paths).expect("load").expect("auth");

    let stats = pull_all(&paths, &Config::defaults(), &mut conn, &auth);

    assert_eq!(
        stats.stalled.as_ref().map(|(seq, _)| *seq),
        Some(8),
        "the stall names the entry that failed, not the page start: {stats:?}"
    );
    assert_eq!(
        sync_state::get(&conn, "ws-org")
            .expect("get")
            .expect("row")
            .pulled_seq,
        7
    );
    assert!(
        store.load(&memory_id(first)).is_ok(),
        "the entry before it stays applied"
    );
    assert!(
        store.load(&memory_id(last)).is_err(),
        "nothing after it is taken as handled"
    );
}
