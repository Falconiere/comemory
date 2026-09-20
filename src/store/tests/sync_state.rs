#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for managed-sync persistence helpers.

use comemory::config::{Config, Paths};
use comemory::domains::memories::{Kind, delete, save};
use comemory::store::{code_sync, connection, memory_repository, sync_manifest, sync_state};
use comemory::utilities::context::Ctx;
use rusqlite::Connection;

fn open_with_sync_state() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    conn.execute_batch(
        "CREATE TABLE sync_state (
            workspace_id TEXT PRIMARY KEY,
            api_url TEXT NOT NULL,
            pulled_seq INTEGER NOT NULL DEFAULT 0,
            pushed_seq INTEGER NOT NULL DEFAULT 0,
            last_sync_at TEXT
        );",
    )
    .expect("schema");
    conn
}

#[test]
fn ensure_and_get_roundtrip() {
    let conn = open_with_sync_state();
    sync_state::ensure(&conn, "ws-1", "https://api.example").expect("ensure");
    let row = sync_state::get(&conn, "ws-1").expect("get").expect("row");
    assert_eq!(row.api_url, "https://api.example");
    assert_eq!(row.pulled_seq, 0);
    assert_eq!(row.pushed_seq, 0);
}

#[test]
fn set_pulled_and_pushed_advance_cursors() {
    let conn = open_with_sync_state();
    sync_state::ensure(&conn, "ws-1", "https://api.example").expect("ensure");
    sync_state::set_pulled(&conn, "ws-1", 12, "2026-09-06T00:00:00Z").expect("pulled");
    sync_state::set_pushed(&conn, "ws-1", 9, "2026-09-06T01:00:00Z").expect("pushed");
    let row = sync_state::get(&conn, "ws-1").expect("get").expect("row");
    assert_eq!(row.pulled_seq, 12);
    assert_eq!(row.pushed_seq, 9);
    assert_eq!(row.last_sync_at.as_deref(), Some("2026-09-06T01:00:00Z"));
}

#[test]
fn list_orders_by_workspace_id() {
    let conn = open_with_sync_state();
    sync_state::ensure(&conn, "ws-b", "https://a").expect("b");
    sync_state::ensure(&conn, "ws-a", "https://a").expect("a");
    let rows = sync_state::list(&conn).expect("list");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].workspace_id, "ws-a");
    assert_eq!(rows[1].workspace_id, "ws-b");
}

#[test]
fn ensure_updates_api_url_on_conflict() {
    let conn = open_with_sync_state();
    sync_state::ensure(&conn, "ws-1", "https://old").expect("ensure");
    sync_state::set_pulled(&conn, "ws-1", 5, "2026-09-01T00:00:00Z").expect("stamp");
    sync_state::ensure(&conn, "ws-1", "https://new").expect("re-ensure");
    let row = sync_state::get(&conn, "ws-1").expect("get").expect("row");
    assert_eq!(row.api_url, "https://new");
    assert_eq!(row.pulled_seq, 5, "cursors must survive api_url rewrite");
}

#[test]
fn get_missing_returns_none() {
    let conn = open_with_sync_state();
    assert!(sync_state::get(&conn, "missing").expect("get").is_none());
}

#[test]
fn a_changed_policy_resets_memory_and_code_cursors_once() {
    let home = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(home.path().join("comemory.db")).expect("db");
    sync_state::ensure(&conn, "ws-1", "https://api.example").expect("ensure");
    sync_state::set_pulled(&conn, "ws-1", 12, "2026-09-20T00:00:00Z").expect("pulled");
    sync_state::set_pushed(&conn, "ws-1", 9, "2026-09-20T00:00:00Z").expect("pushed");
    code_sync::set_cursor(
        &conn,
        "local-label",
        &code_sync::CodeSyncCursor {
            pushed_head: Some("abc".into()),
            pushed_mined_commit: Some("abc".into()),
            pushed_digest: "digest".into(),
            pushed_at: "2026-09-20T00:00:00Z".into(),
        },
    )
    .expect("cursor");

    assert!(sync_state::reconcile_policy(&mut conn, "ws-1", "first").expect("reset"));
    let row = sync_state::get(&conn, "ws-1").expect("get").expect("row");
    assert_eq!((row.pulled_seq, row.pushed_seq), (0, 0));
    assert_eq!(row.last_sync_at, None, "policy reset clears last_sync_at");
    assert!(
        code_sync::cursor(&conn, "local-label")
            .expect("cursor")
            .is_none()
    );

    sync_state::set_pulled(&conn, "ws-1", 4, "2026-09-20T01:00:00Z").expect("pulled");
    assert!(!sync_state::reconcile_policy(&mut conn, "ws-1", "first").expect("same"));
    assert_eq!(
        sync_state::get(&conn, "ws-1")
            .expect("get")
            .expect("row")
            .pulled_seq,
        4
    );
    assert!(sync_state::reconcile_policy(&mut conn, "ws-1", "second").expect("changed"));
    assert_eq!(
        sync_state::get(&conn, "ws-1")
            .expect("get")
            .expect("row")
            .pulled_seq,
        0
    );
}

#[test]
fn repository_labels_and_live_manifest_pairs_follow_real_memory_lifecycle() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).expect("db");
    let (live_id, live_hash) = save_memory(&paths, &cfg, &mut conn, "live decision", "acme/live");
    let (deleted_id, _) = save_memory(&paths, &cfg, &mut conn, "deleted decision", "acme/deleted");
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    delete::run(&mut ctx, &deleted_id).expect("soft delete");

    assert_eq!(
        memory_repository::label(&conn, &live_id).expect("live label"),
        Some("acme/live".into())
    );
    assert_eq!(
        memory_repository::label(&conn, &deleted_id).expect("deleted label"),
        Some("acme/deleted".into())
    );
    assert_eq!(
        memory_repository::label(&conn, "missing00").expect("missing label"),
        None
    );
    assert_eq!(
        sync_manifest::live_repository_hashes(&conn).expect("manifest pairs"),
        vec![(live_hash, "acme/live".into())]
    );
}

fn save_memory(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    body: &str,
    repo: &str,
) -> (String, String) {
    let content_hash = comemory::utilities::digest::sha256_hex(body.trim_end().as_bytes());
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    let response = save::run(
        &mut ctx,
        save::Request {
            body: body.into(),
            title: None,
            kind: Kind::Decision,
            repo: repo.into(),
            tags: Vec::new(),
            author: "review-test".into(),
            quality: 3,
            supersedes: Vec::new(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        },
        false,
        None,
    )
    .expect("save");
    (response.id, content_hash)
}
