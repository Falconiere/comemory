#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/maintenance/erase.rs` (#256, B-5): real data
//! directories, real migrated databases, memories saved and documents
//! indexed through the production cores, and the files on disk read back
//! byte for byte — what an erase removes, what it keeps as the barrier, and
//! that the barrier refuses the erased bytes while new content flows.

use std::fs;
use std::path::{Path, PathBuf};

use comemory::domains::maintenance::erase::{self, Report, Request, Target};
use comemory::domains::sync::replica::contract::Disposition;
use comemory::domains::sync::replica::erasure_manifest;
use comemory::domains::sync::replica::identity::{self, ERASURES_KEY, Ensured, MANIFEST_FILE};
use comemory::prelude::{Error, Result};
use comemory::store::replica_read::{self, Redaction};
use comemory::store::{replica_outbox, replica_redaction};

use crate::domains::sync::replica::accept;
use crate::domains::sync::replica::test_support::{BODY, Home, envelope, upsert};

/// A word no query ever uses, so finding it anywhere after the erase can
/// only mean the erase missed a copy.
const TOKEN: &str = "qzvxErasedToken0417";

fn token_body() -> String {
    format!("{BODY} It also records {TOKEN}, which nothing ever searches for.")
}

fn erase_memory(home: &mut Home, id: &str) -> Result<Report> {
    let mut ctx = home.ctx();
    erase::run(
        &mut ctx,
        Request {
            memory: Some(id.to_string()),
            document: None,
        },
    )
}

/// Every file under `dir` whose bytes contain `needle`.
fn files_holding(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_holding(&path, needle));
        } else if fs::read(&path)
            .unwrap_or_default()
            .windows(needle.len())
            .any(|w| w == needle.as_bytes())
        {
            found.push(path);
        }
    }
    found
}

/// `(op, state, disposition)` of every outbox row for `key`, oldest first.
fn outbox_of(home: &Home, key: &str) -> Vec<(String, String, Option<String>)> {
    let mut statement = home
        .conn
        .prepare(
            "SELECT op, state, disposition FROM replica_operation \
              WHERE entity_key = ?1 ORDER BY created_at, rowid",
        )
        .expect("prepare");
    statement
        .query_map([key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect")
}

/// Every feed position's operation id, oldest first.
fn feed_ids(home: &Home) -> Vec<String> {
    replica_read::page(&home.conn, 0, 100, None)
        .expect("page")
        .into_iter()
        .map(|row| row.operation_id)
        .collect()
}

fn feed_ops(home: &Home, key: &str) -> Vec<String> {
    replica_read::page(&home.conn, 0, 100, None)
        .expect("page")
        .into_iter()
        .filter(|row| row.entity_key == key)
        .map(|row| row.op.as_str().to_string())
        .collect()
}

fn verdict(home: &mut Home, id: &str) {
    let query_id = comemory::utilities::query_id::generate_query_id(
        "durable searchable memory",
        time::OffsetDateTime::now_utc(),
    );
    let mut ctx = home.ctx();
    crate::domains::learning::feedback::run(
        &mut ctx,
        crate::domains::learning::feedback::Request {
            query_id,
            used: vec![id.to_string()],
            irrelevant: Vec::new(),
            used_code: Vec::new(),
            irrelevant_code: Vec::new(),
            source: None,
        },
    )
    .expect("verdict");
}

#[test]
fn a_live_memory_is_tombstoned_then_erased_and_its_text_is_in_no_file() {
    let mut home = Home::new();
    let id = home.save(&token_body(), &["erase"]);
    verdict(&mut home, &id);
    assert!(
        !files_holding(home.paths.data_dir(), TOKEN).is_empty(),
        "the premise: the token is on disk before the erase"
    );

    let report = erase_memory(&mut home, &id).expect("erase");

    assert_eq!(report.kind, "memory");
    assert_eq!(report.key, id);
    assert!(
        report.tombstoned,
        "a live memory's deletion is journalled first"
    );
    assert_eq!(report.payloads_erased, 1, "the one revision the save wrote");
    assert_eq!(report.operations_withdrawn, 1, "the save's pending upload");
    assert!(report.wal_truncated, "no other reader holds the WAL");
    assert!(report.snapshots_with_prior_state.is_empty());
    assert_eq!(
        files_holding(home.paths.data_dir(), TOKEN),
        Vec::<PathBuf>::new(),
        "the token survives in a file under the data directory"
    );
    assert_eq!(
        feed_ops(&home, &id),
        vec!["upsert", "tombstone"],
        "feed rows are kept"
    );
    let revision = replica_read::revision(&home.conn, "memory", &id)
        .expect("revision")
        .expect("the revision is kept");
    assert!(revision.deleted, "and reads as tombstoned");
    assert_eq!(
        outbox_of(&home, &id),
        vec![
            (
                "upsert".to_string(),
                "rejected".to_string(),
                Some("payload_erased".to_string())
            ),
            ("tombstone".to_string(), "pending".to_string(), None),
        ],
        "the upload is withdrawn; the tombstone still goes"
    );
    let verdicts: i64 = home
        .conn
        .query_row(
            "SELECT COUNT(*) FROM feedback_events WHERE memory_id = ?1",
            [&id],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(verdicts, 0, "the verdicts on it are gone");
}

#[test]
fn a_trashed_memory_is_erased_without_a_second_tombstone() {
    let mut home = Home::new();
    let id = home.save(&token_body(), &["erase"]);
    home.delete(&id);
    assert!(!files_holding(&home.paths.trash_dir(), TOKEN).is_empty());

    let report = erase_memory(&mut home, &id).expect("erase");

    assert!(!report.tombstoned, "its deletion is already journalled");
    assert_eq!(feed_ops(&home, &id), vec!["upsert", "tombstone"]);
    assert_eq!(
        files_holding(home.paths.data_dir(), TOKEN),
        Vec::<PathBuf>::new()
    );
}

#[test]
fn an_entity_never_held_is_not_found_and_nothing_is_written() {
    let mut home = Home::new();
    let other = home.save(BODY, &["kept"]);
    let feed_before = feed_ids(&home);
    let pending_before = replica_outbox::count(&home.conn, "pending").expect("count");

    let memory = erase_memory(&mut home, "0badc0de");
    let mut ctx = home.ctx();
    let document = erase::run(
        &mut ctx,
        Request {
            memory: None,
            document: Some("ffffffffffffffffffffffffffffffff".to_string()),
        },
    );

    assert!(matches!(memory, Err(Error::NotFound(_))), "{memory:?}");
    assert!(matches!(document, Err(Error::NotFound(_))), "{document:?}");
    assert_eq!(feed_ids(&home), feed_before);
    assert_eq!(
        replica_outbox::count(&home.conn, "pending").expect("count"),
        pending_before
    );
    assert!(
        crate::domains::memories::MemoryStore::new(home.paths.clone())
            .load(&other)
            .is_ok(),
        "another memory is untouched"
    );
}

#[test]
fn an_erase_on_a_data_directory_without_a_database_creates_none() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = comemory::config::Paths::new(dir.path());
    paths.ensure_dirs().expect("dirs");
    let cfg = comemory::config::Config::defaults();
    let mut ctx = comemory::utilities::context::Ctx::lazy(&paths, &cfg);

    let result = erase::run(
        &mut ctx,
        Request {
            memory: Some("a1b2c3d4".to_string()),
            document: None,
        },
    );

    assert!(matches!(result, Err(Error::NotFound(_))), "{result:?}");
    assert!(
        !paths.db_path().exists(),
        "an erase of nothing writes nothing"
    );
    assert!(
        !paths.data_dir().join(identity::DIR).exists(),
        "not even a stream identity"
    );
}

#[test]
fn a_request_must_name_exactly_one_entity() {
    for request in [
        Request::default(),
        Request {
            memory: Some("a1b2c3d4".to_string()),
            document: Some("f".repeat(32)),
        },
        Request {
            memory: Some("   ".to_string()),
            document: None,
        },
    ] {
        assert!(
            matches!(Target::try_from(request), Err(Error::BadRequest(_))),
            "exactly one entity"
        );
    }
    let target = Target::try_from(Request {
        memory: None,
        document: Some("abc".to_string()),
    })
    .expect("one entity");
    assert_eq!((target.kind(), target.key()), ("document", "abc"));
}

#[test]
fn a_replay_or_a_new_offer_of_the_erased_bytes_is_payload_erased_and_materializes_nothing() {
    let mut author = Home::new();
    let id = author.save(&token_body(), &["erase"]);
    let operation = upsert("op-20260925-aaaaaaaa", &author.payload(&id));
    let mut hub = Home::new();
    let mut ctx = hub.ctx();
    let first = accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("import");
    assert_eq!(first.results[0].disposition, Disposition::Accepted);

    erase_memory(&mut hub, &id).expect("erase on the hub");

    let mut ctx = hub.ctx();
    let replay = accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("replay");
    assert_eq!(
        replay.results[0].disposition,
        Disposition::PayloadErased,
        "a replay of the original import reads the barrier"
    );
    let mut offered_again = operation;
    offered_again.operation_id = "op-20260925-bbbbbbbb".to_string();
    let mut ctx = hub.ctx();
    let again = accept::run(&mut ctx, envelope(vec![offered_again])).expect("offer");
    assert_eq!(again.results[0].disposition, Disposition::PayloadErased);
    assert!(
        crate::domains::memories::MemoryStore::new(hub.paths.clone())
            .load(&id)
            .is_err(),
        "nothing was materialized"
    );
    assert_eq!(
        files_holding(hub.paths.data_dir(), TOKEN),
        Vec::<PathBuf>::new(),
        "and no byte of it was stored again"
    );
}

#[test]
fn the_erased_text_saved_again_is_a_new_revision_shared_normally() {
    let mut home = Home::new();
    let id = home.save(&token_body(), &["erase"]);
    let erased_digest = replica_read::revision(&home.conn, "memory", &id)
        .expect("revision")
        .and_then(|r| r.payload_digest)
        .expect("digest");
    erase_memory(&mut home, &id).expect("erase");

    let again = home.save(&token_body(), &["erase"]);

    assert_eq!(again, id, "the same text keys the same id");
    let digest = replica_read::revision(&home.conn, "memory", &id)
        .expect("revision")
        .and_then(|r| r.payload_digest)
        .expect("the new revision names a payload");
    assert_ne!(digest, erased_digest, "a new `created` makes new bytes");
    assert_eq!(
        replica_redaction::redaction_of(&home.conn, &digest).expect("redaction"),
        None,
        "the new revision keeps its bytes"
    );
    assert_eq!(
        replica_redaction::redaction_of(&home.conn, &erased_digest).expect("redaction"),
        Some(Redaction::Erased),
        "while the old digest stays the barrier"
    );
}

#[test]
fn a_pulled_document_is_erased_locally_without_a_tombstone() {
    use comemory::store::remote_document::{self, Chunk, Revision};
    use comemory::store::replica_journal::{
        self, NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin, stream_epoch,
    };
    let home = Home::new();
    let shared_id = "0f3c9a1e5b7d4c2a8e6f1b3d5a7c9e0f";
    let passage = format!("A pulled passage quoting {TOKEN}.");
    let bytes = format!(r#"{{"shared_id":"{shared_id}","text":"{passage}"}}"#);
    let digest = comemory::utilities::digest::sha256_hex(bytes.as_bytes());
    let tx = home.conn.unchecked_transaction().expect("begin");
    replica_journal::append(
        &tx,
        &stream_epoch(&tx).expect("epoch"),
        &NewOperation {
            operation_id: "op-20260925-pulled01",
            entity_kind: "document_revision",
            entity_key: shared_id,
            op: ReplicaOp::Upsert,
            payload: Some(PayloadRef {
                digest: &digest,
                bytes: &bytes,
            }),
            schema_version: 1,
            repository: Some("Falconiere/comemory"),
            origin: ReplicaOrigin::Sync,
            at: "2026-09-25T10:00:00Z",
        },
    )
    .expect("journal the pulled revision");
    remote_document::replace_revision(
        &tx,
        &Revision {
            repo: "Falconiere/comemory".to_string(),
            shared_id: shared_id.to_string(),
            path: "docs/guides/erase.md".to_string(),
            title: "Erase".to_string(),
            format: "markdown".to_string(),
            revision_hash: "rev".to_string(),
            chunk_count: 1,
        },
        &[Chunk {
            ordinal: 0,
            heading_path: String::new(),
            char_range: (0, 10),
            line_range: (1, 1),
            simhash: 0,
            text: passage,
        }],
        &[],
        "2026-09-25T10:00:00Z",
    )
    .expect("the pulled cache");
    tx.commit().expect("commit");
    let mut home = home;

    let mut ctx = home.ctx();
    let report = erase::run(
        &mut ctx,
        Request {
            memory: None,
            document: Some(shared_id.to_string()),
        },
    )
    .expect("erase");

    assert_eq!(report.kind, "document");
    assert!(!report.tombstoned, "a pulled copy is erased locally only");
    assert_eq!(report.payloads_erased, 1);
    assert_eq!(
        feed_ops(&home, shared_id),
        vec!["upsert"],
        "no tombstone journalled"
    );
    assert!(
        erase_rows_pulled(&home, shared_id).is_empty(),
        "the pulled cache is gone"
    );
    assert_eq!(
        files_holding(home.paths.data_dir(), TOKEN),
        Vec::<PathBuf>::new()
    );
}

fn erase_rows_pulled(home: &Home, shared_id: &str) -> Vec<String> {
    let mut statement = home
        .conn
        .prepare("SELECT repo FROM remote_document WHERE shared_id = ?1")
        .expect("prepare");
    statement
        .query_map([shared_id], |r| r.get(0))
        .expect("query")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect")
}

#[test]
fn the_report_names_every_rollback_snapshot_still_holding_prior_state() {
    let mut home = Home::new();
    let id = home.save(BODY, &["erase"]);
    let data = home.paths.data_dir().to_path_buf();
    for file in [
        "comemory.db.pre-v28.bak",
        "comemory.db.pre-rebuild.bak",
        "comemory.db.pre-restore.bak",
        "comemory.db.rebuild.tmp",
    ] {
        fs::write(data.join(file), b"snapshot").expect("write");
    }
    fs::create_dir_all(data.join("memories.pre-restore")).expect("dir");
    fs::create_dir_all(data.join("backups").join("20260925T100000Z")).expect("dir");

    let report = erase_memory(&mut home, &id).expect("erase");

    let names: Vec<String> = report
        .snapshots_with_prior_state
        .iter()
        .map(|p| {
            Path::new(p)
                .strip_prefix(&data)
                .expect("under the data dir")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        names,
        vec![
            "backups/20260925T100000Z",
            "comemory.db.pre-rebuild.bak",
            "comemory.db.pre-restore.bak",
            "comemory.db.pre-v28.bak",
            "memories.pre-restore",
        ],
        "a temp file is not a snapshot"
    );
}

/// The erasure count the database itself carries.
fn stamped(home: &Home) -> Option<String> {
    home.conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = ?1",
            [ERASURES_KEY],
            |r| r.get(0),
        )
        .ok()
}

fn manifest_of(home: &Home) -> erasure_manifest::Manifest {
    erasure_manifest::read(&identity::file(&home.paths, MANIFEST_FILE))
        .expect("read")
        .expect("erasures.jsonl")
}

#[test]
fn an_erase_appends_its_line_and_stamps_both_counts() {
    let mut home = Home::new();
    let id = home.save(&token_body(), &["erase"]);
    verdict(&mut home, &id);
    let revision = replica_read::revision(&home.conn, "memory", &id)
        .expect("revision")
        .and_then(|r| r.payload_digest)
        .expect("digest");

    let report = erase_memory(&mut home, &id).expect("erase");

    let manifest = manifest_of(&home);
    assert!(manifest.established(1));
    let line = &manifest.lines[0];
    assert_eq!(
        (line.kind.as_str(), line.key.as_str()),
        ("memory", id.as_str())
    );
    assert_eq!(line.digests.len(), report.payloads_erased);
    assert!(line.digests.contains(&revision), "{:?}", line.digests);
    let identity = identity::read(&home.paths)
        .expect("read")
        .expect("identity");
    assert_eq!(identity.erasures, 1);
    assert_eq!(stamped(&home).as_deref(), Some("1"));
    let epoch = home.epoch();
    assert_eq!(
        identity::ensure(&home.paths, &home.cfg, &mut home.conn).expect("ensure"),
        Ensured::Current,
        "an ordinary erase is not a replaced database"
    );
    assert_eq!(home.epoch(), epoch);
}

#[test]
fn an_entity_never_held_records_no_line() {
    let mut home = Home::new();
    home.save(BODY, &["kept"]);

    let missing = erase_memory(&mut home, "0badc0de");

    assert!(matches!(missing, Err(Error::NotFound(_))), "{missing:?}");
    assert!(manifest_of(&home).lines.is_empty());
    let identity = identity::read(&home.paths)
        .expect("read")
        .expect("identity");
    assert_eq!(identity.erasures, 0);
}

/// Env vars telling the second process where to erase, and what.
const CHILD_DIR: &str = "COMEMORY_TEST_ERASE_CHILD_DIR";
const CHILD_IDS: &str = "COMEMORY_TEST_ERASE_CHILD_IDS";

/// Erase every id in `ids` from the data directory at `paths`, each through
/// its own `erase::run` on its own connection, as a CLI process does.
fn erase_all(paths: &comemory::config::Paths, ids: &[String]) {
    let cfg = comemory::config::Config::defaults();
    let mut conn = comemory::store::connection::open(paths.db_path()).expect("open");
    for id in ids {
        let mut ctx = comemory::utilities::context::Ctx::borrowed(paths, &cfg, &mut conn);
        erase::run(
            &mut ctx,
            Request {
                memory: Some(id.clone()),
                document: None,
            },
        )
        .expect("erase");
    }
}

#[test]
fn two_processes_erasing_at_once_keep_the_chain_whole() {
    let wait = std::time::Duration::from_secs(30);
    if let (Some(dir), Some(ids)) = (std::env::var_os(CHILD_DIR), std::env::var(CHILD_IDS).ok()) {
        let paths = comemory::config::Paths::new(PathBuf::from(dir));
        fs::write(paths.data_dir().join("ready"), b"").expect("ready");
        let deadline = std::time::Instant::now() + wait;
        while !paths.data_dir().join("go").exists() {
            assert!(std::time::Instant::now() < deadline, "never told to go");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let ids: Vec<String> = ids.split(',').map(str::to_string).collect();
        erase_all(&paths, &ids);
        return;
    }
    let mut home = Home::new();
    let ids: Vec<String> = (0..8)
        .map(|n| home.save(&format!("{BODY} Concurrent erase number {n}."), &["erase"]))
        .collect();
    let (mine, theirs) = ids.split_at(4);
    let test = format!(
        "{}::two_processes_erasing_at_once_keep_the_chain_whole",
        module_path!().trim_start_matches("comemory::")
    );
    let mut child = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", &test, "--nocapture"])
        .env(CHILD_DIR, home.paths.data_dir())
        .env(CHILD_IDS, theirs.join(","))
        .spawn()
        .expect("spawn the second process");
    let deadline = std::time::Instant::now() + wait;
    while !home.paths.data_dir().join("ready").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the second process never started"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    fs::write(home.paths.data_dir().join("go"), b"").expect("go");

    erase_all(&home.paths, mine);
    let status = child.wait().expect("wait");
    assert!(status.success(), "the second process failed: {status}");

    let manifest = manifest_of(&home);
    assert!(manifest.intact, "two erasers broke the chain");
    let mut keys: Vec<String> = manifest.lines.iter().map(|l| l.key.clone()).collect();
    keys.sort();
    let mut expected = ids.clone();
    expected.sort();
    assert_eq!(keys, expected, "one line per erase, from both processes");
    let identity = identity::read(&home.paths)
        .expect("read")
        .expect("identity");
    assert_eq!(identity.erasures, 8);
    assert_eq!(stamped(&home).as_deref(), Some("8"));
    assert_eq!(
        identity::ensure(&home.paths, &home.cfg, &mut home.conn).expect("ensure"),
        Ensured::Current
    );
}
