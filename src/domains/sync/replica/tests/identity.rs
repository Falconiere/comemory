#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/sync/replica/identity.rs` (#256, B-4): real
//! data directories, real migrated databases, memories saved, imported and
//! erased through the production cores, and a real `.bak` (`VACUUM INTO`)
//! copied over `comemory.db` with the engine closed — the manual copy the
//! identity exists to catch.

use std::fs;
use std::path::{Path, PathBuf};

use comemory::config::Paths;
use comemory::domains::maintenance::erase::{self, Request};
use comemory::domains::sync::AuthFile;
use comemory::domains::sync::drain::drain;
use comemory::domains::sync::drain::session::{Legs, Mode};
use comemory::domains::sync::replica::contract::{Disposition, Operation};
use comemory::domains::sync::replica::erasure_manifest::{self, Manifest};
use comemory::domains::sync::replica::identity::{
    self, CREATED, ERASURES_KEY, Ensured, Identity, MANIFEST_FILE, REPLACED,
};
use comemory::store::replica_read::Redaction;
use comemory::store::replica_redaction::{self, Reach};
use comemory::store::{connection, replica_device, replica_journal};
use rusqlite::Connection;
use serde_json::json;

use crate::domains::sync::drain::test_support::{LiveEngine, client_of};
use crate::domains::sync::replica::accept;
use crate::domains::sync::replica::test_support::{BODY, Home, envelope, tombstone, upsert};
use crate::test_common::serve_state;

/// A word no query ever uses: finding it after the merge means the copy
/// brought erased bytes back and the merge missed them.
const TOKEN: &str = "qzvxIdentityToken0925";

fn ensure(home: &mut Home) -> Ensured {
    identity::ensure(&home.paths, &home.cfg, &mut home.conn).expect("ensure")
}

fn identity_of(home: &Home) -> Identity {
    identity::read(&home.paths)
        .expect("read")
        .expect("identity.json")
}

fn manifest_of(home: &Home) -> Option<Manifest> {
    erasure_manifest::read(&identity::file(&home.paths, MANIFEST_FILE)).expect("read")
}

/// The erasure count the database itself carries.
fn stamped(conn: &Connection) -> Option<String> {
    conn.query_row(
        "SELECT value FROM schema_meta WHERE key = ?1",
        [ERASURES_KEY],
        |r| r.get(0),
    )
    .ok()
}

/// Every payload row whose bytes still hold `needle`.
fn payloads_holding(conn: &Connection, needle: &str) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM replica_payload WHERE instr(bytes, ?1) > 0",
        [needle],
        |r| r.get(0),
    )
    .expect("count")
}

/// Put `epoch` on the database, as a database from another stream carries.
fn forge_epoch(conn: &Connection, epoch: &str) {
    conn.execute("UPDATE replica_stream SET epoch = ?1", [epoch])
        .expect("forge");
}

/// A self-contained snapshot of the live database, as `VACUUM INTO` writes
/// every `.bak` this engine makes.
fn snapshot(home: &Home) -> PathBuf {
    let bak = home.paths.data_dir().join("comemory.db.hand.bak");
    home.conn
        .execute("VACUUM INTO ?1", [bak.to_str().expect("utf8")])
        .expect("vacuum into");
    bak
}

/// Close the engine's connection, copy `bak` over `comemory.db` the way an
/// operator would by hand, and open it again.
fn copy_over(home: &mut Home, bak: &Path) {
    let open = std::mem::replace(
        &mut home.conn,
        Connection::open_in_memory().expect("placeholder"),
    );
    open.close().expect("close the live database");
    let db = home.paths.db_path();
    for side in ["-wal", "-shm"] {
        let path = PathBuf::from(format!("{}{side}", db.display()));
        if path.exists() {
            fs::remove_file(&path).expect("remove a stale side file");
        }
    }
    fs::copy(bak, &db).expect("copy the .bak over comemory.db");
    home.conn = connection::open(&db).expect("reopen");
}

/// Import one memory `author` saves with `body` into `hub` under
/// `operation_id`; returns the operation and its payload digest.
fn import(hub: &mut Home, operation_id: &str, body: &str) -> (Operation, String) {
    let mut author = Home::new();
    let id = author.save(body, &["erase"]);
    let operation = upsert(operation_id, &author.payload(&id));
    let digest = operation.payload_digest.clone().expect("digest");
    let mut ctx = hub.ctx();
    let accepted = accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("import");
    assert_eq!(accepted.results[0].disposition, Disposition::Accepted);
    (operation, digest)
}

/// A hub holding one imported memory whose body carries [`TOKEN`], its
/// identity already written. Returns the hub, the import and its digest.
fn hub_with_token() -> (Home, Operation, String) {
    let mut hub = Home::new();
    let body = format!("{BODY} It also records {TOKEN}, which nothing searches for.");
    let (operation, digest) = import(&mut hub, "op-20260925-aaaaaaaa", &body);
    assert_eq!(ensure(&mut hub), Ensured::Created);
    (hub, operation, digest)
}

fn erase_memory(home: &mut Home, id: &str) {
    let mut ctx = home.ctx();
    erase::run(
        &mut ctx,
        Request {
            memory: Some(id.to_string()),
            document: None,
        },
    )
    .expect("erase");
}

#[test]
fn the_first_ensure_writes_both_files_from_the_database() {
    let mut home = Home::new();
    let kept = home.save(BODY, &["kept"]);
    let gone = home.save("A memory whose payload was already erased.", &["gone"]);
    let digests = replica_redaction::redact(
        &home.conn,
        Reach::Entity("memory", &gone),
        "2026-09-25T10:00:00Z",
    )
    .expect("redact");
    assert_eq!(digests.len(), 1, "the premise: one payload already erased");

    assert_eq!(ensure(&mut home), Ensured::Created);

    let identity = identity_of(&home);
    assert_eq!(identity.epoch, home.epoch());
    assert_eq!(
        identity.device_id,
        replica_device::id(&home.conn).expect("device")
    );
    assert_eq!(identity.erasures, 1);
    assert_eq!(identity.epochs.len(), 1);
    assert_eq!(identity.epochs[0].reason, CREATED);
    let manifest = manifest_of(&home).expect("erasures.jsonl");
    assert!(manifest.established(1));
    assert_eq!(
        (
            manifest.lines[0].kind.as_str(),
            manifest.lines[0].key.as_str()
        ),
        ("memory", gone.as_str())
    );
    assert_eq!(manifest.lines[0].digests, digests);
    assert!(manifest.lines.iter().all(|l| l.key != kept));
    assert_eq!(stamped(&home.conn).as_deref(), Some("1"));
    assert_eq!(ensure(&mut home), Ensured::Current, "written once");
}

#[test]
fn a_database_carrying_another_epoch_is_reepoched_and_keeps_the_device() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    ensure(&mut home);
    let (first, device) = (home.epoch(), identity_of(&home).device_id);
    forge_epoch(&home.conn, &"f".repeat(32));

    let Ensured::Reepoched { epoch, merged } = ensure(&mut home) else {
        panic!("a foreign epoch must be re-epoched");
    };

    assert_eq!(merged, 0);
    assert_ne!(epoch, first);
    assert_ne!(epoch, "f".repeat(32));
    assert_eq!(home.epoch(), epoch);
    let identity = identity_of(&home);
    assert_eq!(identity.epoch, epoch);
    assert_eq!(identity.device_id, device);
    assert_eq!(replica_device::id(&home.conn).expect("device"), device);
    let reasons: Vec<&str> = identity.epochs.iter().map(|e| e.reason.as_str()).collect();
    assert_eq!(reasons, vec![CREATED, REPLACED]);
    assert_eq!(ensure(&mut home), Ensured::Current, "repaired once");
}

#[test]
fn a_bak_copied_over_the_database_is_reepoched_and_the_erasure_merged() {
    let (mut hub, operation, digest) = hub_with_token();
    let before = hub.epoch();
    let bak = snapshot(&hub);
    erase_memory(&mut hub, &operation.entity_key);
    assert_eq!(identity_of(&hub).erasures, 1);

    copy_over(&mut hub, &bak);
    assert_eq!(
        payloads_holding(&hub.conn, TOKEN),
        1,
        "the premise: the copy brought the erased bytes back"
    );
    assert_eq!(hub.epoch(), before, "under the epoch peers' cursors hold");

    let Ensured::Reepoched { epoch, merged } = ensure(&mut hub) else {
        panic!("a database from before an erase must be re-epoched and merged");
    };

    assert_eq!(merged, 1);
    assert_ne!(epoch, before);
    assert_eq!(hub.epoch(), epoch);
    assert_eq!(identity_of(&hub).epoch, epoch);
    assert_eq!(
        replica_redaction::redaction_of(&hub.conn, &digest).expect("redaction"),
        Some(Redaction::Erased)
    );
    assert_eq!(
        payloads_holding(&hub.conn, TOKEN),
        0,
        "no erased byte survives"
    );
    let mut ctx = hub.ctx();
    let replay = accept::run(&mut ctx, envelope(vec![operation])).expect("replay");
    assert_eq!(
        replay.results[0].disposition,
        Disposition::PayloadErased,
        "a replay of the original import reads the merged barrier"
    );
    assert_eq!(stamped(&hub.conn).as_deref(), Some("1"));
    assert_eq!(ensure(&mut hub), Ensured::Current, "merged once");
}

#[test]
fn a_bak_with_the_manifest_gone_is_reepoched_once_and_reports_erasure_unknown() {
    let (mut hub, operation, _digest) = hub_with_token();
    let before = hub.epoch();
    let bak = snapshot(&hub);
    erase_memory(&mut hub, &operation.entity_key);
    copy_over(&mut hub, &bak);
    fs::remove_file(identity::file(&hub.paths, MANIFEST_FILE)).expect("the manifest is gone");

    let Ensured::ErasureUnknown { epoch, merged } = ensure(&mut hub) else {
        panic!("a replaced database without its manifest must report erasure_unknown");
    };

    assert_eq!(merged, 0);
    assert_ne!(
        epoch, before,
        "stream identity is never kept through a manual copy"
    );
    assert_eq!(hub.epoch(), epoch);
    // The database now carries the count the identity holds, so the next read
    // serves the new epoch instead of minting another on every request.
    assert_eq!(stamped(&hub.conn).as_deref(), Some("1"));
    assert_eq!(ensure(&mut hub), Ensured::Current);
    assert_eq!(hub.epoch(), epoch);
    assert_eq!(identity_of(&hub).epochs.len(), 2);
}

#[test]
fn a_torn_manifest_merges_only_the_lines_that_verify() {
    let (mut hub, first, first_digest) = hub_with_token();
    let (second, second_digest) = import(
        &mut hub,
        "op-20260925-bbbbbbbb",
        "A second shared memory, erased after the first.",
    );
    let bak = snapshot(&hub);
    erase_memory(&mut hub, &first.entity_key);
    erase_memory(&mut hub, &second.entity_key);
    copy_over(&mut hub, &bak);
    let manifest = identity::file(&hub.paths, MANIFEST_FILE);
    let bytes = fs::read(&manifest).expect("bytes");
    fs::write(&manifest, &bytes[..bytes.len() - 10]).expect("tear the last line");

    let Ensured::ErasureUnknown { merged, .. } = ensure(&mut hub) else {
        panic!("a torn manifest is not established");
    };

    assert_eq!(merged, 1, "the line before the tear still verifies");
    let redaction = |d: &str| replica_redaction::redaction_of(&hub.conn, d).expect("redaction");
    assert_eq!(redaction(&first_digest), Some(Redaction::Erased));
    assert_eq!(
        redaction(&second_digest),
        None,
        "unknown, so not guessed at"
    );
}

/// The epoch the database behind `paths` carries.
fn db_epoch(paths: &Paths) -> String {
    let conn = connection::open(paths.db_path()).expect("open");
    replica_journal::stream_epoch(&conn).expect("epoch")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_replica_route_runs_ensure_before_its_core() {
    let session = serve_state::session(false);
    let paths = Paths::new(session.home.path());
    let first = serve_state::send(&session, "GET", "/api/v1/sync/replica/manifest", None).await;
    assert_eq!(first.status, 200, "{}", first.text);
    assert!(
        identity::read(&paths).expect("read").is_some(),
        "the first replica read writes the identity"
    );
    let operation =
        serde_json::to_value(tombstone("op-20260925-cccccccc", "a1b2c3d4")).expect("op");
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
    ];
    for (i, (method, path, body)) in routes.into_iter().enumerate() {
        let forged = format!("{i:0>32}");
        forge_epoch(&connection::open(paths.db_path()).expect("open"), &forged);
        let before = identity::read(&paths).expect("read").expect("identity");

        let resp = serve_state::send(&session, method, path, body).await;

        let after = identity::read(&paths).expect("read").expect("identity");
        let served = db_epoch(&paths);
        assert_ne!(
            served, forged,
            "{path} ran its core over a replaced database ({}): {}",
            resp.status, resp.text
        );
        assert_eq!(after.epoch, served, "{path}");
        assert_eq!(
            after.epochs.len(),
            before.epochs.len() + 1,
            "{path} re-epoched exactly once"
        );
        if let Some(epoch) = resp.json["data"]["stream_epoch"].as_str() {
            assert_eq!(epoch, served, "{path} answered under the new epoch");
        }
    }
}

#[test]
fn a_drain_runs_ensure_before_it_opens_a_session() {
    let engine = LiveEngine::start();
    let mut client = client_of(&engine);
    client.save(BODY, &["sync"]);
    assert_eq!(ensure(&mut client), Ensured::Created);
    let forged = "e".repeat(32);
    forge_epoch(&client.conn, &forged);
    let auth = AuthFile::load(&client.paths).expect("load").expect("auth");
    let (paths, cfg) = (client.paths.clone(), client.cfg.clone());

    drain(
        &paths,
        &cfg,
        &mut client.conn,
        &auth,
        (Mode::Manual, Legs::Both),
    )
    .expect("drain");

    assert_ne!(client.epoch(), forged);
    let identity = identity_of(&client);
    assert_eq!(identity.epoch, client.epoch());
    assert_eq!(
        identity.epochs.last().map(|e| e.reason.as_str()),
        Some(REPLACED)
    );
}
