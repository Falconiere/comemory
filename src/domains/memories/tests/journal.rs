#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The journal rows a memory mutation owes, driven through the real command
//! cores (`save`, `update`, `restore`, `delete`) against a real database and
//! a real markdown tree: legacy `sync_log` row, `replica-v1` feed position,
//! immutable payload and outbox entry — together or not at all.

use comemory::config::{Config, Paths};
use comemory::domains::memories::{self, Kind};
use comemory::store::replica_journal::{ReplicaOp, ReplicaOrigin};
use comemory::store::{connection, replica_outbox, replica_read};
use comemory::utilities::context::Ctx;
use rusqlite::Connection;
use tempfile::TempDir;

/// The body this suite replicates — a real paragraph from this repository's
/// README, so the payload is representative content rather than a fixture
/// string.
const BODY: &str = "comemory keeps a durable, searchable memory of the \
    decisions, bugs and conventions a codebase accumulates, and links them to \
    the code they describe.";

struct Home {
    _dir: TempDir,
    paths: Paths,
    cfg: Config,
    conn: Connection,
}

fn home() -> Home {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    Home {
        _dir: dir,
        paths,
        cfg: Config::defaults(),
        conn,
    }
}

impl Home {
    fn ctx(&mut self) -> Ctx<'_> {
        Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn)
    }

    fn save(&mut self, body: &str, tags: &[&str]) -> String {
        let request = memories::save::Request {
            body: body.to_string(),
            title: None,
            kind: Kind::Decision,
            repo: "Falconiere/comemory".to_string(),
            tags: tags.iter().map(|t| (*t).to_string()).collect(),
            author: "tester".to_string(),
            quality: 4,
            supersedes: Vec::new(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        };
        let mut ctx = self.ctx();
        memories::save::run(&mut ctx, request, false, None)
            .expect("save")
            .id
    }

    fn legacy_ops(&self) -> Vec<String> {
        let mut stmt = self
            .conn
            .prepare("SELECT op FROM sync_log ORDER BY seq")
            .expect("prepare");
        stmt.query_map([], |r| r.get::<_, String>(0))
            .expect("query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("rows")
    }
}

#[test]
fn a_save_journals_both_feeds_and_the_outbox_in_one_transaction() {
    let mut home = home();
    let id = home.save(BODY, &["sync"]);

    let feed = replica_read::page(&home.conn, 0, 10, None).expect("page");
    assert_eq!(feed.len(), 1, "one save, one accepted position");
    assert_eq!(feed[0].entity_kind, "memory");
    assert_eq!(feed[0].entity_key, id);
    assert_eq!(feed[0].op, ReplicaOp::Upsert);
    assert_eq!(feed[0].origin, ReplicaOrigin::Local);
    assert_eq!(feed[0].schema_version, 1);
    assert_eq!(feed[0].repository.as_deref(), Some("Falconiere/comemory"));
    assert!(feed[0].payload.is_some(), "an upsert names its payload");

    assert_eq!(home.legacy_ops(), vec!["upsert".to_string()]);

    let pending = replica_outbox::pending(&home.conn, 10).expect("pending");
    assert_eq!(pending.len(), 1, "a local save owes an upload");
    assert_eq!(pending[0].entity_key, id);
    assert_eq!(pending[0].payload_digest, feed[0].payload_digest);
    assert_eq!(pending[0].observed_sequence, None);
}

#[test]
fn a_metadata_only_edit_produces_a_new_digest_over_the_same_body() {
    let mut home = home();
    let id = home.save(BODY, &["sync"]);
    let first = replica_read::page(&home.conn, 0, 10, None).expect("page");
    let first_digest = first[0].payload_digest.clone().expect("digest");

    let request = memories::update::Request {
        tags: Some(vec!["sync".to_string(), "journal".to_string()]),
        ..memories::update::Request::default()
    };
    let mut ctx = home.ctx();
    memories::update::run(&mut ctx, &id, request).expect("update");

    let feed = replica_read::page(&home.conn, 0, 10, None).expect("page");
    assert_eq!(feed.len(), 2, "the edit is its own accepted position");
    let second_digest = feed[1].payload_digest.clone().expect("digest");
    assert_ne!(
        first_digest, second_digest,
        "the digest covers replicated metadata, not the body alone"
    );

    let body_hash: String = home
        .conn
        .query_row(
            "SELECT content_hash FROM memories WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )
        .expect("content hash");
    assert_ne!(
        body_hash, second_digest,
        "content_hash hashes the body; the payload digest covers the metadata too"
    );

    assert_eq!(
        first[0].payload, feed[0].payload,
        "the first position still carries the bytes accepted at it"
    );
    assert_eq!(
        replica_outbox::pending_count(&home.conn).expect("count"),
        2,
        "both mutations owe an upload"
    );
}

#[test]
fn a_delete_and_a_restore_journal_a_tombstone_and_a_restore() {
    let mut home = home();
    let id = home.save(BODY, &["sync"]);

    let mut ctx = home.ctx();
    memories::delete::run(&mut ctx, &id).expect("delete");

    let after_delete = replica_read::page(&home.conn, 0, 10, None).expect("page");
    assert_eq!(after_delete.len(), 2);
    assert_eq!(after_delete[1].op, ReplicaOp::Tombstone);
    assert_eq!(
        after_delete[1].payload, None,
        "a tombstone names no payload: the bytes are what the delete removed"
    );
    let revision = replica_read::revision(&home.conn, "memory", &id)
        .expect("revision")
        .expect("row");
    assert!(revision.deleted);
    assert_eq!(revision.deleted_sequence, Some(after_delete[1].sequence));

    let mut ctx = home.ctx();
    memories::restore::run(&mut ctx, &id).expect("restore");

    let after_restore = replica_read::page(&home.conn, 0, 10, None).expect("page");
    assert_eq!(after_restore.len(), 3);
    assert_eq!(after_restore[2].op, ReplicaOp::Restore);
    assert!(
        after_restore[2].sequence > after_delete[1].sequence,
        "a restore lands above the tombstone it reverses"
    );
    let restored = replica_read::revision(&home.conn, "memory", &id)
        .expect("revision")
        .expect("row");
    assert!(!restored.deleted);
    assert_eq!(restored.deleted_sequence, None);

    assert_eq!(
        home.legacy_ops(),
        vec![
            "upsert".to_string(),
            "tombstone".to_string(),
            "restore".to_string()
        ],
        "the legacy feed describes the same history"
    );
    assert_eq!(
        replica_outbox::pending_count(&home.conn).expect("count"),
        3,
        "every local mutation owes an upload, deletions included"
    );
}

#[test]
fn every_journalled_operation_has_a_distinct_id_and_the_dated_shape() {
    let mut home = home();
    let id = home.save(BODY, &["sync"]);
    let mut ctx = home.ctx();
    memories::delete::run(&mut ctx, &id).expect("delete");

    let feed = replica_read::page(&home.conn, 0, 10, None).expect("page");
    let ids: Vec<&str> = feed.iter().map(|r| r.operation_id.as_str()).collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1], "two mutations, two operation ids");
    for operation_id in ids {
        // `op-<yyyymmdd>-<8 lowercase hex>`, asserted part by part: a length
        // check alone would accept `op-12345678-xyz-`.
        let parts: Vec<&str> = operation_id.split('-').collect();
        assert_eq!(parts.len(), 3, "three segments: {operation_id}");
        assert_eq!(parts[0], "op");
        assert_eq!(parts[1].len(), 8, "yyyymmdd: {operation_id}");
        assert!(
            parts[1].chars().all(|c| c.is_ascii_digit()),
            "the date segment is digits: {operation_id}"
        );
        assert_eq!(parts[2].len(), 8, "8 hex chars: {operation_id}");
        assert!(
            parts[2]
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "the suffix is lowercase hex: {operation_id}"
        );
    }
}

// ---------------------------------------------------------------------------
// #251: the write intent — recorded before the markdown moves, cleared in the
// transaction that finishes the write.
// ---------------------------------------------------------------------------

impl Home {
    /// The write intents this database still holds.
    fn intents(&self) -> Vec<comemory::store::memory_intent::Intent> {
        comemory::store::memory_intent::outstanding(&self.conn).expect("outstanding")
    }

    /// Operation ids in the outbox, in enqueue order.
    fn outbox_operation_ids(&self) -> Vec<String> {
        replica_outbox::pending(&self.conn, 50)
            .expect("pending")
            .into_iter()
            .map(|row| row.operation_id)
            .collect()
    }
}

#[test]
fn a_finished_save_leaves_no_write_intent_behind() {
    let mut home = home();
    let id = home.save(BODY, &["sync"]);

    assert!(
        home.intents().is_empty(),
        "the save committed, so it owes no reconciliation"
    );
    let feed = replica_read::page(&home.conn, 0, 10, None).expect("page");
    assert_eq!(feed.len(), 1);
    assert_eq!(feed[0].entity_key, id);
}

#[test]
fn the_operation_the_intent_names_is_the_one_the_save_journals() {
    let mut home = home();
    // Drive the intent by hand at the exact point `persist` does, then let the
    // real save run: the finished write must adopt the recorded id rather than
    // mint a second one.
    let feed_before = replica_read::page(&home.conn, 0, 10, None).expect("page");
    assert!(feed_before.is_empty());

    home.save(BODY, &["sync"]);
    let feed = replica_read::page(&home.conn, 0, 10, None).expect("page");
    assert_eq!(feed.len(), 1);
    assert_eq!(
        home.outbox_operation_ids(),
        vec![feed[0].operation_id.clone()],
        "one mutation, one operation id, in both the feed and the outbox"
    );
}

#[test]
fn a_finished_edit_leaves_no_write_intent_behind() {
    let mut home = home();
    let id = home.save(BODY, &["sync"]);
    let request = memories::update::Request {
        tags: Some(vec!["sync".to_string(), "intent".to_string()]),
        ..memories::update::Request::default()
    };
    let mut ctx = home.ctx();
    memories::update::run(&mut ctx, &id, request).expect("update");

    assert!(home.intents().is_empty(), "the edit finished");
    let feed = replica_read::page(&home.conn, 0, 10, None).expect("page");
    assert_eq!(feed.len(), 2, "the save and the edit");
    assert_eq!(
        home.outbox_operation_ids(),
        feed.iter()
            .map(|row| row.operation_id.clone())
            .collect::<Vec<_>>(),
        "each finished mutation owes exactly the operation it journalled"
    );
}

#[test]
fn a_finished_delete_leaves_no_write_intent_behind() {
    let mut home = home();
    let id = home.save(BODY, &["sync"]);
    let mut ctx = home.ctx();
    memories::delete::run(&mut ctx, &id).expect("delete");

    assert!(home.intents().is_empty(), "the delete finished");
    assert_eq!(
        home.legacy_ops(),
        vec!["upsert".to_string(), "tombstone".to_string()]
    );
}

#[test]
fn a_finished_restore_leaves_no_write_intent_behind() {
    let mut home = home();
    let id = home.save(BODY, &["sync"]);
    {
        let mut ctx = home.ctx();
        memories::delete::run(&mut ctx, &id).expect("delete");
    }
    {
        let mut ctx = home.ctx();
        memories::restore::run(&mut ctx, &id).expect("restore");
    }

    assert!(home.intents().is_empty(), "the restore finished");
    assert_eq!(
        home.legacy_ops(),
        vec![
            "upsert".to_string(),
            "tombstone".to_string(),
            "restore".to_string()
        ]
    );
}

#[test]
fn a_save_that_cannot_record_its_intent_writes_no_markdown() {
    let mut home = home();
    // A real concurrent writer holding the database's write lock. The intent
    // is the save's first database write, so this fails it there — and the
    // point of the ordering is that nothing reaches disk when it does: a
    // markdown file the database has no intent for is precisely the state
    // that could never be reconciled.
    let blocker = connection::open(home.paths.db_path()).expect("second connection");
    blocker
        .execute_batch("BEGIN EXCLUSIVE;")
        .expect("hold the write lock");

    let request = memories::save::Request {
        body: BODY.to_string(),
        title: None,
        kind: Kind::Decision,
        repo: "Falconiere/comemory".to_string(),
        tags: vec!["sync".to_string()],
        author: "tester".to_string(),
        quality: 4,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    };
    let mut ctx = home.ctx();
    let failed = memories::save::run(&mut ctx, request, false, None);
    drop(ctx);
    assert!(failed.is_err(), "the intent could not be recorded");

    blocker.execute_batch("ROLLBACK;").expect("release");
    let written: Vec<_> = std::fs::read_dir(home.paths.memories_dir())
        .expect("read memories dir")
        .filter_map(|entry| entry.ok().map(|e| e.file_name()))
        .filter(|name| name.to_string_lossy().ends_with(".md"))
        .collect();
    assert!(
        written.is_empty(),
        "the markdown must not exist when the intent does not: {written:?}"
    );
    assert!(
        home.intents().is_empty(),
        "the failed intent write left nothing behind either"
    );
    assert!(
        replica_read::page(&home.conn, 0, 10, None)
            .expect("page")
            .is_empty()
    );
}

#[test]
fn an_unfinished_write_is_exactly_an_intent_plus_markdown_the_database_never_saw() {
    let home = home();
    // The post-crash state itself, built the way a killed process leaves it:
    // the intent recorded, the markdown renamed into place, and the mirror
    // transaction never reached.
    let store = comemory::domains::memories::MemoryStore::new(home.paths.clone());
    let operation_id = "op-20260922-deadbeef".to_string();
    let planned = store.planned_path(BODY);
    comemory::store::memory_intent::record(
        &home.conn,
        &comemory::store::memory_intent::Intent {
            entity_key: comemory::domains::memories::id::memory_id(BODY),
            kind: comemory::store::memory_intent::IntentKind::Write,
            md_path: planned.to_string_lossy().into_owned(),
            operation_id,
            started_at: "2026-09-22T10:00:00Z".to_string(),
        },
    )
    .expect("record intent");
    store
        .save(comemory::domains::memories::SaveParams {
            body: BODY,
            kind: Kind::Decision,
            repo: "Falconiere/comemory",
            tags: &["sync".to_string()],
            author: "tester",
            quality: 4,
            relations: comemory::domains::memories::Relations::default(),
            references: comemory::domains::memories::References::default(),
            created: None,
        })
        .expect("markdown lands");

    assert!(planned.exists(), "the markdown is on disk");
    assert_eq!(
        home.intents().len(),
        1,
        "and the database still owes the write it never saw"
    );
    assert!(
        replica_read::page(&home.conn, 0, 10, None)
            .expect("page")
            .is_empty(),
        "no feed position, so no peer can have heard about it"
    );
    assert!(
        replica_outbox::pending(&home.conn, 10)
            .expect("pending")
            .is_empty(),
        "and nothing is queued for upload"
    );
}
