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
        assert!(
            operation_id.starts_with("op-"),
            "operation ids carry the op- prefix: {operation_id}"
        );
        assert_eq!(operation_id.len(), 20, "op-<yyyymmdd>-<8hex>");
    }
}
