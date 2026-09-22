#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

use time::OffsetDateTime;

use comemory::config::{Config, Paths};
use comemory::domains::memories::frontmatter::{References, Relations};
use comemory::domains::memories::id::memory_id;
use comemory::domains::memories::save;
use comemory::domains::memories::{Kind, MemoryStore};
use comemory::domains::sync::exchange;
use comemory::store::connection;
use comemory::store::sync_log::{self, SyncOp};
use comemory::utilities::context::Ctx;
use comemory::utilities::digest::sha256_hex;

fn wire_frontmatter(body: &str) -> exchange::WireFrontmatter {
    let id = memory_id(body);
    let content_hash = sha256_hex(body.trim_end().as_bytes());
    exchange::WireFrontmatter {
        id,
        kind: Kind::Note,
        repo: "demo".into(),
        tags: vec!["sync".into()],
        created: OffsetDateTime::now_utc(),
        quality: 3,
        schema: 1,
        content_hash,
        references: References::default(),
        relations: Relations::default(),
    }
}

fn import_entry(body: &str, op: SyncOp) -> exchange::ImportEntry {
    let fm = wire_frontmatter(body);
    exchange::ImportEntry {
        op,
        id: fm.id.clone(),
        content_hash: fm.content_hash.clone(),
        at: "2026-09-06T12:00:00Z".into(),
        record: Some(exchange::SyncRecord {
            frontmatter: fm,
            body: body.to_string(),
            vector: None,
        }),
    }
}

#[test]
fn import_new_upsert_is_accepted() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let body = "sync import creates a brand-new memory";
    let resp = exchange::import::run(
        &mut ctx,
        exchange::ImportRequest {
            cursor: 0,
            entries: vec![import_entry(body, SyncOp::Upsert)],
        },
        Some("device-user"),
    )
    .expect("import");

    assert_eq!(resp.results.len(), 1);
    assert_eq!(resp.results[0].status, exchange::ImportStatus::Accepted);
    assert!(resp.results[0].seq.is_some());

    let store = MemoryStore::new(paths.clone());
    let rec = store.load(&resp.results[0].id).expect("live memory");
    assert_eq!(rec.frontmatter.author, "device-user");
}

#[test]
fn tombstone_unknown_id_is_deleted_status() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let resp = exchange::import::run(
        &mut ctx,
        exchange::ImportRequest {
            cursor: 0,
            entries: vec![exchange::ImportEntry {
                op: SyncOp::Tombstone,
                id: "deadbeef".into(),
                content_hash: "00".repeat(32),
                at: "2026-09-06T12:00:00Z".into(),
                record: None,
            }],
        },
        None,
    )
    .expect("import");

    assert_eq!(resp.results[0].status, exchange::ImportStatus::Deleted);
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sync_log WHERE op = 'tombstone'",
            [],
            |r| r.get(0),
        )
        .expect("count tombstones");
    assert_eq!(count, 1);
}

#[test]
fn id_collision_rejects_live_hash_mismatch() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let body = "live memory for id-collision helper";
    save::run(
        &mut ctx,
        save::Request {
            body: body.to_string(),
            title: None,
            kind: Kind::Note,
            repo: "demo".into(),
            tags: Vec::new(),
            author: String::new(),
            quality: 3,
            supersedes: Vec::new(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        },
        false,
        None,
    )
    .expect("seed save");
    let id = memory_id(body);

    assert!(
        exchange::import_state::id_collision_for_test(&paths, &id, &"ff".repeat(32))
            .expect("probe"),
        "live memory hash must differ from a forged wire hash"
    );
}

#[test]
fn id_collision_treats_an_unreadable_local_copy_as_repairable() {
    // A hand-mangled live file under the id: not a collision the pull must
    // stop for — the wire record is allowed to repair it (the CLI/HTTP save
    // is stricter and refuses; see `MemoryStore::prior`).
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    std::fs::write(
        paths.memories_dir().join("deadbeef-broken.md"),
        "---\nid: [\n---\nbody\n",
    )
    .expect("plant broken file");

    assert!(
        !exchange::import_state::id_collision_for_test(&paths, "deadbeef", &"ff".repeat(32))
            .expect("an unparsable local copy is not an error for the import"),
        "unreadable copy must not read as a collision"
    );
}

#[test]
fn secret_detected_blocks_upsert() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let aws_key = format!("AKIA{}{}", "IOSFODNN7", "EXAMPLE");
    let body = format!("leaked {aws_key} key in body");
    let resp = exchange::import::run(
        &mut ctx,
        exchange::ImportRequest {
            cursor: 0,
            entries: vec![import_entry(&body, SyncOp::Upsert)],
        },
        None,
    )
    .expect("import");

    assert_eq!(
        resp.results[0].status,
        exchange::ImportStatus::SecretDetected
    );
    assert_eq!(resp.results[0].reason.as_deref(), Some("aws-access-key-id"));
}

#[test]
fn stale_upsert_after_tombstone() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let body = "memory that will be deleted then replayed";
    save::run(
        &mut ctx,
        save::Request {
            body: body.to_string(),
            title: None,
            kind: Kind::Note,
            repo: "demo".into(),
            tags: Vec::new(),
            author: String::new(),
            quality: 3,
            supersedes: Vec::new(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        },
        false,
        None,
    )
    .expect("seed save");
    let id = memory_id(body);
    crate::domains::memories::delete::run(&mut ctx, &id).expect("delete");

    let tomb_seq = {
        let conn = ctx.conn().expect("conn");
        sync_log::latest_tombstone_seq(conn, &id)
            .expect("tombstone seq")
            .expect("tombstone written")
    };

    let resp = exchange::import::run(
        &mut ctx,
        exchange::ImportRequest {
            cursor: tomb_seq - 1,
            entries: vec![import_entry(body, SyncOp::Upsert)],
        },
        None,
    )
    .expect("import");

    assert_eq!(resp.results[0].status, exchange::ImportStatus::Stale);
}

#[test]
fn an_imported_tombstone_journals_both_feeds_in_the_delete_s_own_transaction() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let body = "An imported deletion must never outlive the record of it.";
    save::run(
        &mut ctx,
        save::Request {
            body: body.to_string(),
            title: None,
            kind: Kind::Note,
            repo: "demo".into(),
            tags: Vec::new(),
            author: String::new(),
            quality: 3,
            supersedes: Vec::new(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        },
        false,
        None,
    )
    .expect("seed save");
    let id = memory_id(body);

    let resp = exchange::import::run(
        &mut ctx,
        exchange::ImportRequest {
            cursor: 0,
            entries: vec![import_entry(body, SyncOp::Tombstone)],
        },
        None,
    )
    .expect("import");
    assert_eq!(resp.results[0].status, exchange::ImportStatus::Accepted);

    let conn = ctx.conn().expect("conn");
    // The markdown is gone, and BOTH feeds recorded why — written inside the
    // delete's transaction, so no crash window can drop the tombstone and let
    // the next pull resurrect the memory.
    assert!(
        MemoryStore::new(paths.clone()).load(&id).is_err(),
        "the import deleted the memory"
    );
    assert!(
        sync_log::latest_tombstone_seq(conn, &id)
            .expect("legacy seq")
            .is_some(),
        "the legacy feed carries the tombstone"
    );
    let revision = comemory::store::replica_read::revision(conn, "memory", &id)
        .expect("revision")
        .expect("the replica feed carries it too");
    assert!(revision.deleted);
    assert_eq!(
        revision.deleted_sequence,
        Some(revision.sequence),
        "the tombstone records its own position"
    );

    let deleted_at: Option<String> = conn
        .query_row(
            "SELECT deleted_at FROM memories WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )
        .expect("mirror row");
    assert!(deleted_at.is_some(), "the mirror row is soft-deleted");
}

// ---------------------------------------------------------------------------
// #251: the legacy wire and the replica wire reach the SAME verdict on the
// same vector. Both go through `domains::sync::vector_rule`; these cases pin
// the legacy half, and `replica::accept`'s tests pin the other.
// ---------------------------------------------------------------------------

/// A real base64 wire vector of `dims` little-endian `f32`s.
fn wire_vector(model: &str, dims: u32) -> exchange::SyncVector {
    let mut values = vec![0.0_f32; dims as usize];
    if let Some(first) = values.first_mut() {
        *first = 1.0;
    }
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    exchange::SyncVector {
        model: model.to_string(),
        dims,
        f32: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
    }
}

/// An upsert entry carrying `vector`.
fn import_entry_with_vector(body: &str, vector: exchange::SyncVector) -> exchange::ImportEntry {
    let mut entry = import_entry(body, SyncOp::Upsert);
    if let Some(record) = entry.record.as_mut() {
        record.vector = Some(vector);
    }
    entry
}

#[test]
fn the_legacy_wire_stores_a_compatible_vector_once() {
    let body = "connection pooling keeps the handshake off the hot path";
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let model = comemory::store::schema_meta::memory_vector_model(&conn).expect("model");

    for _ in 0..2 {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        exchange::import::run(
            &mut ctx,
            exchange::ImportRequest {
                cursor: 0,
                entries: vec![import_entry_with_vector(body, wire_vector(&model, 1024))],
            },
            Some("device-user"),
        )
        .expect("import");
    }

    let id = memory_id(body);
    assert!(
        comemory::store::vector::memory_embedding_blob(&conn, &id)
            .expect("blob")
            .is_some()
    );
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM memory_vec", [], |r| r.get(0))
        .expect("count");
    assert_eq!(rows, 1, "a replay replaces rather than duplicates");
    assert_eq!(
        comemory::store::needs_embedding::pending_count(&conn).expect("count"),
        0
    );
}

#[test]
fn the_legacy_wire_drops_a_foreign_model_and_records_it_as_needing_an_embedding() {
    let body = "advisory locks serialize the writer without blocking readers";
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        exchange::import::run(
            &mut ctx,
            exchange::ImportRequest {
                cursor: 0,
                entries: vec![import_entry_with_vector(
                    body,
                    wire_vector("text-embedding-3-small", 1024),
                )],
            },
            Some("device-user"),
        )
        .expect("import");
    }

    let id = memory_id(body);
    assert!(
        MemoryStore::new(paths.clone()).load(&id).is_ok(),
        "the memory text lands regardless"
    );
    let pending = comemory::store::needs_embedding::pending(&conn).expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].memory_id, id);
    assert_eq!(
        pending[0].reason,
        comemory::store::needs_embedding::Reason::Model
    );
    assert_eq!(pending[0].model.as_deref(), Some("text-embedding-3-small"));
    assert!(
        comemory::store::vector::memory_embedding_blob(&conn, &id)
            .expect("blob")
            .is_none()
    );
}

#[test]
fn the_legacy_wire_records_an_absent_vector_as_a_backlog_entry() {
    let body = "the writer reservation is taken before the index walk";
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        exchange::import::run(
            &mut ctx,
            exchange::ImportRequest {
                cursor: 0,
                entries: vec![import_entry(body, SyncOp::Upsert)],
            },
            Some("device-user"),
        )
        .expect("import");
    }

    let pending = comemory::store::needs_embedding::pending(&conn).expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].memory_id, memory_id(body));
    assert_eq!(
        pending[0].reason,
        comemory::store::needs_embedding::Reason::Absent
    );
}
