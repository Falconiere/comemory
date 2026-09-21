#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! Shared fixture for the `replica-v1` core tests: a real data directory, a
//! real migrated database, and real saved memories to build operations from.
//!
//! Declared once from `domains::sync::replica` as `test_support`, so every
//! `tests/` module here uses one copy of it.

use crate::config::{Config, Paths};
use crate::domains::memories::replica_payload::MemoryPayloadV1;
use crate::domains::memories::{self, Kind, MemoryStore};
use crate::domains::sync::replica::contract::{CursorRef, ImportRequest, Operation, PROTOCOL};
use crate::store::connection;
use crate::store::replica_journal::{ReplicaOp, stream_epoch};
use crate::utilities::context::Ctx;
use rusqlite::Connection;
use tempfile::TempDir;

/// Real prose from this repository's README, so payloads carry representative
/// content rather than a fixture string.
pub const BODY: &str = "comemory keeps a durable, searchable memory of the decisions, bugs and \
     conventions a codebase accumulates, and links them to the code they describe.";

/// A throwaway data directory with its own migrated database.
pub struct Home {
    _dir: TempDir,
    pub paths: Paths,
    pub cfg: Config,
    pub conn: Connection,
}

impl Home {
    /// A fresh home with the schema applied.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::new(dir.path());
        paths.ensure_dirs().expect("ensure_dirs");
        let conn = connection::open(paths.db_path()).expect("open db");
        Self {
            _dir: dir,
            paths,
            cfg: Config::defaults(),
            conn,
        }
    }

    /// A context borrowing this home's connection.
    pub fn ctx(&mut self) -> Ctx<'_> {
        Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn)
    }

    /// This database's stream epoch.
    pub fn epoch(&self) -> String {
        stream_epoch(&self.conn).expect("epoch")
    }

    /// Save one real memory through the production core and return its id.
    pub fn save(&mut self, body: &str, tags: &[&str]) -> String {
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

    /// The canonical payload of a saved memory, as a peer would send it.
    pub fn payload(&self, id: &str) -> MemoryPayloadV1 {
        let record = MemoryStore::new(self.paths.clone())
            .load(id)
            .expect("load record");
        MemoryPayloadV1::from_record(&record).expect("payload")
    }
}

/// An upsert operation carrying `payload` under `operation_id`.
pub fn upsert(operation_id: &str, payload: &MemoryPayloadV1) -> Operation {
    let (bytes, digest) = payload.canonical().expect("canonical");
    Operation {
        operation_id: operation_id.to_string(),
        entity_kind: "memory".to_string(),
        entity_key: payload.id.clone(),
        op: ReplicaOp::Upsert,
        schema_version: 1,
        payload_digest: Some(digest),
        payload: Some(serde_json::from_str(&bytes).expect("payload json")),
        observed_sequence: None,
        repository: Some("Falconiere/comemory".to_string()),
    }
}

/// A tombstone operation for `entity_key`.
pub fn tombstone(operation_id: &str, entity_key: &str) -> Operation {
    Operation {
        operation_id: operation_id.to_string(),
        entity_kind: "memory".to_string(),
        entity_key: entity_key.to_string(),
        op: ReplicaOp::Tombstone,
        schema_version: 1,
        payload_digest: None,
        payload: None,
        observed_sequence: None,
        repository: None,
    }
}

/// An import envelope carrying `operations`.
pub fn envelope(operations: Vec<Operation>) -> ImportRequest {
    ImportRequest {
        protocol: PROTOCOL.to_string(),
        cursor: None,
        operations,
        workspace_id: None,
    }
}

/// An import envelope with a cursor at `sequence` under `epoch`.
pub fn envelope_at(operations: Vec<Operation>, epoch: &str, sequence: i64) -> ImportRequest {
    ImportRequest {
        cursor: Some(CursorRef {
            stream_epoch: epoch.to_string(),
            sequence,
        }),
        ..envelope(operations)
    }
}

#[test]
fn the_fixture_saves_a_real_memory_and_builds_the_operation_a_peer_would_send() {
    let mut home = Home::new();
    let id = home.save(BODY, &["sync"]);
    let payload = home.payload(&id);
    let operation = upsert("op-20260921-aaaaaaaa", &payload);

    assert_eq!(operation.entity_key, id);
    assert_eq!(
        operation.payload_digest,
        Some(payload.canonical().expect("canonical").1)
    );
    assert_eq!(home.epoch().len(), 32);
    assert!(operation.payload.is_some());
}
