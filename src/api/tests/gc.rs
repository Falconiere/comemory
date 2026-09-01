#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/gc.rs`. Calls `api::gc::run` directly against a
//! `Ctx::lazy` opened on a temp data-dir, proving the trash sweep,
//! telemetry retention purge, and — critically — that a fresh data dir with
//! no prior `comemory.db` is never touched by `run` (`cli::gc::run` is
//! byte-compat tested against CLI stdout in `tests/cli__gc.rs`; the HTTP
//! route lives in `tests/serve__routes__maint__prune.rs`). The second half
//! drives real `save` → `delete` → aged-mtime → `gc` cycles over a
//! `Ctx::borrowed` and proves a reaped file's mirror rows go with it, a
//! zombie row (file already gone, `deleted_at` past the window) is purged,
//! and a live memory or a fresh trash entry is never touched.

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use time::{Duration, OffsetDateTime};

fn db_path(home: &tempfile::TempDir) -> std::path::PathBuf {
    home.path().join("comemory.db")
}

fn seed_telemetry(home: &tempfile::TempDir) {
    std::fs::create_dir_all(home.path()).expect("create data dir");
    let conn = comemory::store::connection::open(db_path(home)).expect("open + migrate db");
    let now = OffsetDateTime::now_utc();
    let old =
        comemory::store::memory_row::iso_format(now - Duration::days(100)).expect("old stamp");
    let fresh =
        comemory::store::memory_row::iso_format(now - Duration::days(1)).expect("fresh stamp");
    for (qid, at) in [("q-old", &old), ("q-new", &fresh)] {
        conn.execute(
            "INSERT INTO retrieval_log(query_id, query, returned_ids, at) \
             VALUES (?1, 'some query', '[]', ?2)",
            rusqlite::params![qid, at],
        )
        .expect("insert retrieval_log row");
    }
}

#[test]
fn run_on_a_fresh_data_dir_never_creates_the_db() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    assert!(!db_path(&home).exists(), "db must not exist yet");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = api::gc::run(&mut ctx, api::gc::Request {}).expect("gc run");

    assert_eq!(resp.removed, 0);
    assert_eq!(resp.log_rows, 0);
    assert_eq!(resp.event_rows, 0);
    assert_eq!(resp.bytes_freed, 0);
    assert!(
        !db_path(&home).exists(),
        "gc on a fresh dir must not create comemory.db"
    );
}

#[test]
fn run_sweeps_old_telemetry_when_the_db_already_exists() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    seed_telemetry(&home);

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = api::gc::run(&mut ctx, api::gc::Request {}).expect("gc run");

    assert_eq!(resp.removed, 0);
    assert_eq!(resp.log_rows, 1, "one old retrieval_log row swept");
    assert_eq!(resp.event_rows, 0);
    assert_eq!(resp.bytes_freed, 0, "no trash swept in this run");

    let conn = rusqlite::Connection::open_with_flags(
        db_path(&home),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("reopen db read-only");
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM retrieval_log", [], |r| r.get(0))
        .expect("count retrieval_log");
    assert_eq!(remaining, 1, "fresh row must survive");

    let (removed, log_rows, bytes_freed): (i64, i64, i64) = conn
        .query_row(
            "SELECT removed, log_rows, bytes_freed FROM gc_runs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("gc_runs row written when the db already exists");
    assert_eq!(removed, 0);
    assert_eq!(log_rows, 1);
    assert_eq!(bytes_freed, 0);
}

#[test]
fn run_sweeps_trash_entries_older_than_thirty_days() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let trash = paths.trash_dir();
    std::fs::create_dir_all(&trash).expect("create trash dir");
    let old = trash.join("11111111-old.md");
    let fresh = trash.join("22222222-fresh.md");
    std::fs::write(&old, "old").expect("write old trash entry");
    std::fs::write(&fresh, "fresh").expect("write fresh trash entry");
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(&old)
        .expect("reopen old trash entry");
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_hours(31 * 24))
        .expect("backdate mtime");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = api::gc::run(&mut ctx, api::gc::Request {}).expect("gc run");

    assert_eq!(resp.removed, 1);
    assert_eq!(resp.bytes_freed, 3, "the removed \"old\" file is 3 bytes");
    assert!(!old.exists(), "old trash entry must be deleted");
    assert!(fresh.exists(), "fresh trash entry must be kept");
    assert!(
        !db_path(&home).exists(),
        "sweeping trash alone (no seeded db) must not create comemory.db"
    );
}

/// A migrated data dir plus a borrowed connection, the shape the purge
/// tests drive `api::gc::run` through (a `Ctx::borrowed`, like the CLI).
fn open_store(home: &tempfile::TempDir) -> (Paths, Config, rusqlite::Connection) {
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let conn = comemory::store::connection::open(paths.db_path()).expect("open + migrate db");
    (paths, Config::defaults(), conn)
}

/// Save one note through the real `api::save::run`, returning its id.
fn save_note(paths: &Paths, cfg: &Config, conn: &mut rusqlite::Connection, body: &str) -> String {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    api::save::run(
        &mut ctx,
        api::save::Request {
            body: body.to_string(),
            title: None,
            kind: comemory::memory::Kind::Note,
            repo: "demo".to_string(),
            tags: vec!["gc".to_string()],
            author: "tester".to_string(),
            quality: 3,
            supersedes: Vec::new(),
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

/// Soft-delete `id` through the real `api::delete::run` and return the
/// `.trash/` path the file landed at.
fn soft_delete(
    paths: &Paths,
    cfg: &Config,
    conn: &mut rusqlite::Connection,
    id: &str,
) -> std::path::PathBuf {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    api::delete::run(&mut ctx, id).expect("soft delete");
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    let page = api::trash::run(
        &mut ctx,
        api::trash::Request {
            limit: 0,
            offset: 0,
        },
    )
    .expect("trash listing");
    let row = page
        .items
        .iter()
        .find(|r| r.id == id)
        .expect("soft-deleted memory is listed in the trash");
    std::path::PathBuf::from(row.path.clone().expect("trashed file is on disk"))
}

/// Age `path`'s mtime past the 30-day trash window with a real mtime
/// rewrite, never a faked clock.
fn backdate(path: &std::path::Path) {
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("reopen trashed file");
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_hours(31 * 24))
        .expect("backdate mtime");
}

fn gc(paths: &Paths, cfg: &Config, conn: &mut rusqlite::Connection) -> api::gc::Response {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    api::gc::run(&mut ctx, api::gc::Request {}).expect("gc run")
}

fn trash_ids(paths: &Paths, cfg: &Config, conn: &mut rusqlite::Connection) -> Vec<String> {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    api::trash::run(
        &mut ctx,
        api::trash::Request {
            limit: 0,
            offset: 0,
        },
    )
    .expect("trash listing")
    .items
    .into_iter()
    .map(|r| r.id)
    .collect()
}

fn count_by_id(conn: &rusqlite::Connection, sql: &str, id: &str) -> i64 {
    conn.query_row(sql, [id], |r| r.get(0)).expect("count")
}

#[test]
fn run_purges_the_mirror_rows_behind_a_reaped_trash_file() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_store(&home);
    let id = save_note(&paths, &cfg, &mut conn, "reaped together with its rows");
    let trashed = soft_delete(&paths, &cfg, &mut conn, &id);
    backdate(&trashed);
    assert_eq!(
        count_by_id(&conn, "SELECT COUNT(*) FROM memories WHERE id = ?1", &id),
        1,
        "the soft delete keeps the row"
    );

    let resp = gc(&paths, &cfg, &mut conn);

    assert_eq!(resp.removed, 1, "the aged file was reaped");
    assert_eq!(resp.purged_rows, 1, "its row went with it");
    assert!(!trashed.exists(), "trash file unlinked");
    for (label, sql) in [
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
            "edges",
            "SELECT COUNT(*) FROM edges WHERE (src_kind = 'memory' AND src_id = ?1) \
             OR (dst_kind = 'memory' AND dst_id = ?1)",
        ),
    ] {
        assert_eq!(
            count_by_id(&conn, sql, &id),
            0,
            "{label}: no zombie row after gc"
        );
    }
    assert!(
        trash_ids(&paths, &cfg, &mut conn).is_empty(),
        "the trash listing no longer shows the reaped memory"
    );
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let stats = api::stats::run(&mut ctx, api::stats::Request { repo: None }).expect("stats");
    assert_eq!(stats.trashed, 0, "stats.trashed no longer counts it");

    let second = gc(&paths, &cfg, &mut conn);
    assert_eq!(
        (second.removed, second.purged_rows),
        (0, 0),
        "a second sweep finds nothing left"
    );
}

#[test]
fn run_purges_a_zombie_row_whose_trash_file_is_already_gone() {
    // The state every pre-purge `gc` left behind: the file unlinked by an
    // earlier sweep, the `memories` row still soft-deleted and past the
    // window. An upgrade must self-heal on its next sweep.
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_store(&home);
    let id = save_note(&paths, &cfg, &mut conn, "zombie left by an older gc");
    let trashed = soft_delete(&paths, &cfg, &mut conn, &id);
    std::fs::remove_file(&trashed).expect("simulate the earlier sweep's unlink");
    let old =
        comemory::store::memory_row::iso_format(OffsetDateTime::now_utc() - Duration::days(45))
            .expect("old stamp");
    conn.execute(
        "UPDATE memories SET deleted_at = ?1 WHERE id = ?2",
        rusqlite::params![old, id],
    )
    .expect("age deleted_at");
    assert_eq!(
        trash_ids(&paths, &cfg, &mut conn),
        vec![id.clone()],
        "listed as a zombie"
    );

    let resp = gc(&paths, &cfg, &mut conn);

    assert_eq!(resp.removed, 0, "no file left to reap");
    assert_eq!(resp.bytes_freed, 0);
    assert_eq!(resp.purged_rows, 1, "the zombie row is purged");
    assert_eq!(
        count_by_id(&conn, "SELECT COUNT(*) FROM memories WHERE id = ?1", &id),
        0
    );
    assert!(trash_ids(&paths, &cfg, &mut conn).is_empty());
}

#[test]
fn run_leaves_live_memories_and_fresh_trash_entries_alone() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_store(&home);
    let live = save_note(&paths, &cfg, &mut conn, "a live memory gc must never touch");
    let fresh = save_note(&paths, &cfg, &mut conn, "deleted today, inside the window");
    let fresh_path = soft_delete(&paths, &cfg, &mut conn, &fresh);
    // An old `deleted_at` on a row whose trash file is still on disk is not
    // a zombie: the file's mtime is the clock, and it is fresh.
    let old =
        comemory::store::memory_row::iso_format(OffsetDateTime::now_utc() - Duration::days(45))
            .expect("old stamp");
    conn.execute(
        "UPDATE memories SET deleted_at = ?1 WHERE id = ?2",
        rusqlite::params![old, fresh],
    )
    .expect("age deleted_at");

    let resp = gc(&paths, &cfg, &mut conn);

    assert_eq!((resp.removed, resp.purged_rows), (0, 0));
    assert!(fresh_path.exists(), "fresh trash entry kept");
    assert_eq!(
        count_by_id(
            &conn,
            "SELECT COUNT(*) FROM memories WHERE id = ?1 AND deleted_at IS NULL",
            &live
        ),
        1,
        "live row untouched"
    );
    assert_eq!(
        count_by_id(
            &conn,
            "SELECT COUNT(*) FROM memory_fts WHERE memory_id = ?1",
            &live
        ),
        1,
        "live FTS row untouched"
    );
    assert_eq!(trash_ids(&paths, &cfg, &mut conn), vec![fresh]);
}
