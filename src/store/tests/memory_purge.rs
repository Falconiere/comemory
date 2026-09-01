#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`purge_memory`] / [`expired_deleted_ids`] against a real migrated
//! `comemory.db`, populated through the real writers: `api::save::run`,
//! `api::delete::run` (the soft delete), `api::feedback::run`,
//! `store::code_ref::upsert`, `store::vector::insert_memory`.

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::memory::{Kind, Ref, References};
use comemory::stats::feedback::generate_query_id;
use comemory::store::memory_purge::{expired_deleted_ids, purge_memory};
use comemory::store::{code_ref, connection, fts, memory_row, vector};
use rusqlite::Connection;
use time::{Duration, OffsetDateTime};

fn open(home: &std::path::Path) -> (Paths, Config, Connection) {
    let paths = Paths::new(home);
    paths.ensure_dirs().expect("ensure_dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

fn save(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    body: &str,
    supersedes: &[&str],
) -> String {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    api::save::run(
        &mut ctx,
        api::save::Request {
            body: body.to_string(),
            title: None,
            kind: Kind::Note,
            repo: "demo".to_string(),
            tags: vec!["purge".to_string()],
            author: "tester".to_string(),
            quality: 3,
            supersedes: supersedes.iter().map(|s| (*s).to_string()).collect(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        },
        false,
        None,
    )
    .expect("save")
    .id
}

fn soft_delete(paths: &Paths, cfg: &Config, conn: &mut Connection, id: &str) {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    api::delete::run(&mut ctx, id).expect("soft delete");
}

fn count(conn: &Connection, sql: &str, id: &str) -> i64 {
    conn.query_row(sql, [id], |r| r.get(0)).expect("count")
}

/// Every table a purge must clear, as `(label, COUNT(*) query bound to the
/// memory id)`.
fn dependent_counts(conn: &Connection, id: &str) -> Vec<(&'static str, i64)> {
    [
        ("memories", "SELECT COUNT(*) FROM memories WHERE id = ?1"),
        (
            "memory_tags",
            "SELECT COUNT(*) FROM memory_tags WHERE memory_id = ?1",
        ),
        (
            "memory_fts",
            "SELECT COUNT(*) FROM memory_fts WHERE memory_id = ?1",
        ),
        (
            "memory_vec",
            "SELECT COUNT(*) FROM memory_vec WHERE memory_id = ?1",
        ),
        (
            "edges",
            "SELECT COUNT(*) FROM edges WHERE (src_kind = 'memory' AND src_id = ?1) \
             OR (dst_kind = 'memory' AND dst_id = ?1)",
        ),
        (
            "code_ref",
            "SELECT COUNT(*) FROM code_ref WHERE memory_id = ?1",
        ),
        (
            "feedback",
            "SELECT COUNT(*) FROM feedback WHERE memory_id = ?1",
        ),
        (
            "feedback_events",
            "SELECT COUNT(*) FROM feedback_events WHERE memory_id = ?1 AND target_kind = 'memory'",
        ),
    ]
    .into_iter()
    .map(|(label, sql)| (label, count(conn, sql, id)))
    .collect()
}

#[test]
fn purge_clears_every_mirror_row_of_a_soft_deleted_memory() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open(home.path());
    let id = save(
        &paths,
        &cfg,
        &mut conn,
        "the connection pool leaks under load",
        &[],
    );

    // Real feedback for the memory while it is live: a counter row plus a
    // memory-target event.
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::feedback::run(
            &mut ctx,
            api::feedback::Request {
                query_id: generate_query_id("pool leak", OffsetDateTime::now_utc()),
                used: vec![id.clone()],
                irrelevant: Vec::new(),
                used_code: Vec::new(),
                irrelevant_code: Vec::new(),
            },
        )
        .expect("record feedback");
    }
    // A code-target event whose text-encoded rowid has the exact shape of
    // this memory's id: it must survive the purge untouched.
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind) \
         VALUES ('q-20260101-deadbeef', ?1, 'used', '2026-01-01T00:00:00.000000000Z', 'code')",
        [&id],
    )
    .expect("insert code-target event");
    // A pinned code reference (the soft delete leaves `code_ref` alone).
    let refs = References {
        files: vec![Ref::new("demo:src/pool.rs")],
        symbols: Vec::new(),
    };
    let stamp = memory_row::iso_format(OffsetDateTime::now_utc()).expect("stamp");
    code_ref::upsert(&conn, &id, &refs, &stamp).expect("code_ref upsert");

    soft_delete(&paths, &cfg, &mut conn, &id);
    // The soft delete drops `memory_fts` itself (`cli::delete::mirror_soft_delete`),
    // so the index row is put back through the real writer: that is the
    // state a store carries when its trashed rows were indexed by an older
    // version, and it is what the purge has to clear.
    fts::index_memory(&conn, &id, "the connection pool leaks under load", "purge")
        .expect("re-index the trashed id");
    // A dangling incoming edge minted AFTER the soft delete, and a vector
    // row a caller embedded against the trashed id — both must go with it.
    let superseder = save(
        &paths,
        &cfg,
        &mut conn,
        "the pool now uses a bounded semaphore",
        &[&id],
    );
    let dim = vector::dim_memory(&conn).expect("dim");
    vector::insert_memory(&conn, &id, &vec![0.5; dim]).expect("insert vec row");

    let before = dependent_counts(&conn, &id);
    for (label, n) in &before {
        assert!(
            *n >= 1,
            "{label}: expected at least one row before purge, got {n}"
        );
    }

    assert!(
        purge_memory(&mut conn, &id).expect("purge"),
        "soft-deleted row purged"
    );

    for (label, n) in dependent_counts(&conn, &id) {
        assert_eq!(n, 0, "{label}: rows must be gone after purge");
    }
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM feedback_events WHERE memory_id = ?1 AND target_kind = 'code'",
            &id
        ),
        1,
        "the code-target event sharing the id shape must survive"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM memories WHERE id = ?1 AND deleted_at IS NULL",
            &superseder
        ),
        1,
        "the superseder stays live"
    );
    assert!(
        !purge_memory(&mut conn, &id).expect("second purge"),
        "a second purge of the same id finds nothing"
    );
}

#[test]
fn purge_never_touches_a_live_memory() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open(home.path());
    let id = save(&paths, &cfg, &mut conn, "still very much in use", &[]);
    let before = dependent_counts(&conn, &id);

    assert!(
        !purge_memory(&mut conn, &id).expect("purge live"),
        "a live row is refused"
    );

    assert_eq!(dependent_counts(&conn, &id), before, "nothing changed");
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM memories WHERE id = ?1 AND deleted_at IS NULL",
            &id
        ),
        1
    );
    assert!(
        !purge_memory(&mut conn, "00000000").expect("purge unknown"),
        "an unknown id is a no-op"
    );
}

#[test]
fn expired_deleted_ids_reports_only_rows_past_the_window() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open(home.path());
    let old = save(&paths, &cfg, &mut conn, "deleted long ago", &[]);
    let fresh = save(&paths, &cfg, &mut conn, "deleted just now", &[]);
    let live = save(&paths, &cfg, &mut conn, "never deleted", &[]);
    soft_delete(&paths, &cfg, &mut conn, &old);
    soft_delete(&paths, &cfg, &mut conn, &fresh);
    let stamp =
        memory_row::iso_format(OffsetDateTime::now_utc() - Duration::days(40)).expect("stamp");
    conn.execute(
        "UPDATE memories SET deleted_at = ?1 WHERE id = ?2",
        rusqlite::params![stamp, old],
    )
    .expect("age deleted_at");

    let expired = expired_deleted_ids(&conn, 30).expect("expired ids");
    assert_eq!(
        expired,
        vec![old.clone()],
        "only the 40-day-old deletion is past a 30-day window"
    );
    assert!(
        !expired.contains(&fresh) && !expired.contains(&live),
        "fresh deletion and live row excluded"
    );
    assert!(
        expired_deleted_ids(&conn, 60)
            .expect("wider window")
            .is_empty(),
        "a 60-day window keeps the 40-day-old deletion"
    );
}
