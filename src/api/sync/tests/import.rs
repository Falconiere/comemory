#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

use time::OffsetDateTime;

use comemory::api::{self, Ctx, save, sync};
use comemory::config::{Config, Paths};
use comemory::memory::frontmatter::{References, Relations};
use comemory::memory::id::{memory_id, sha256_hex};
use comemory::memory::{Kind, MemoryStore};
use comemory::store::connection;
use comemory::store::sync_log::{self, SyncOp};

fn wire_frontmatter(body: &str) -> sync::WireFrontmatter {
    let id = memory_id(body);
    let content_hash = sha256_hex(body.trim_end().as_bytes());
    sync::WireFrontmatter {
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

fn import_entry(body: &str, op: SyncOp) -> sync::ImportEntry {
    let fm = wire_frontmatter(body);
    sync::ImportEntry {
        op,
        id: fm.id.clone(),
        content_hash: fm.content_hash.clone(),
        at: "2026-09-06T12:00:00Z".into(),
        record: Some(sync::SyncRecord {
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
    let resp = sync::import::run(
        &mut ctx,
        sync::ImportRequest {
            cursor: 0,
            entries: vec![import_entry(body, SyncOp::Upsert)],
        },
        Some("device-user"),
    )
    .expect("import");

    assert_eq!(resp.results.len(), 1);
    assert_eq!(resp.results[0].status, sync::ImportStatus::Accepted);
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

    let resp = sync::import::run(
        &mut ctx,
        sync::ImportRequest {
            cursor: 0,
            entries: vec![sync::ImportEntry {
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

    assert_eq!(resp.results[0].status, sync::ImportStatus::Deleted);
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
        sync::import_state::id_collision_for_test(&paths, &id, &"ff".repeat(32)).expect("probe"),
        "live memory hash must differ from a forged wire hash"
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
    let resp = sync::import::run(
        &mut ctx,
        sync::ImportRequest {
            cursor: 0,
            entries: vec![import_entry(&body, SyncOp::Upsert)],
        },
        None,
    )
    .expect("import");

    assert_eq!(resp.results[0].status, sync::ImportStatus::SecretDetected);
    assert_eq!(resp.results[0].reason.as_deref(), Some("aws-access-key"));
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
    api::delete::run(&mut ctx, &id).expect("delete");

    let tomb_seq = {
        let conn = ctx.conn().expect("conn");
        sync_log::latest_tombstone_seq(conn, &id)
            .expect("tombstone seq")
            .expect("tombstone written")
    };

    let resp = sync::import::run(
        &mut ctx,
        sync::ImportRequest {
            cursor: tomb_seq - 1,
            entries: vec![import_entry(body, SyncOp::Upsert)],
        },
        None,
    )
    .expect("import");

    assert_eq!(resp.results[0].status, sync::ImportStatus::Stale);
}
