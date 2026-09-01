#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for `crate::store::migrate::preflight`, all against real SQLite
//! databases built by the real migration chain (via
//! `store::tests::migrate_2::build_legacy_db`) or replayed to head via
//! `connection::open` itself. No mocks.

use std::path::Path;

use comemory::store::connection;
use comemory::store::migrate::tests_2::{build_legacy_db, migration_chain};
use comemory::store::migrate::{self, list};
use rusqlite::Connection;
use tempfile::tempdir;

/// Every `(key, value)` row currently in `schema_meta`, ordered by key —
/// used to prove a refused/failed preflight leaves the table byte-for-byte
/// unchanged.
fn schema_meta_snapshot(conn: &Connection) -> Vec<(String, String)> {
    conn.prepare("SELECT key, value FROM schema_meta ORDER BY key")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows")
}

/// True when a row with `id` exists in `memories`.
fn has_memory(conn: &Connection, id: &str) -> bool {
    conn.query_row("SELECT count(*) FROM memories WHERE id = ?1", [id], |r| {
        r.get::<_, i64>(0)
    })
    .expect("count probe")
        == 1
}

/// Regression test for the bug this whole feature exists to prevent: a
/// real, fully-migrated (v13) database — built by the real migration
/// chain via `connection::open`, not hand-assembled — must NOT be refused
/// on a subsequent open. A version-string check would have waved this
/// through by accident; a naive "applied keys == MIGRATIONS keys" check
/// would have refused it (see the module doc: `0001` records no marker,
/// `0004`/`0005` each add a second non-migration marker).
#[test]
fn real_current_database_is_not_refused() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");

    // First open: fresh DB, migrates all the way to head.
    let conn = connection::open(&db).expect("first open migrates to head");
    drop(conn);

    // Second open: everything is already applied. Must succeed, and must
    // not write a snapshot (applied == expected is the zero-I/O branch).
    let conn2 = connection::open(&db).expect("second open on a current DB must not be refused");
    let version: String = conn2
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |r| r.get(0),
        )
        .expect("version");
    assert_eq!(version, migrate::CURRENT_VERSION);

    let bak_count = std::fs::read_dir(dir.path())
        .expect("read data dir")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".bak"))
        .count();
    assert_eq!(bak_count, 0, "an already-current DB must create no .bak");
}

/// Insert a bogus `0016_future` marker directly, simulating a database
/// written by a newer comemory. `version_value` lets the caller pin
/// `schema_meta.version` at either `"16"` (a completed future upgrade) or
/// `"15"` (a crash between the last migration and the version write) —
/// both must be refused identically, since preflight never reads
/// `version` at all.
fn inject_future_marker(conn: &Connection, version_value: &str) {
    conn.execute(
        "INSERT INTO schema_meta(key, value) VALUES('0016_future', '1')",
        [],
    )
    .expect("seed future marker");
    conn.execute(
        "UPDATE schema_meta SET value = ?1 WHERE key = 'version'",
        [version_value],
    )
    .expect("pin version");
}

/// Shared body for both crash-window variants: build a real v13 DB, inject
/// the unknown marker, reopen, and assert the refusal plus the unchanged
/// `schema_meta`.
fn assert_future_marker_is_refused(version_value: &str) {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    {
        let conn = connection::open(&db).expect("build a real v15 db");
        inject_future_marker(&conn, version_value);
    }

    let before = {
        let raw = Connection::open(&db).expect("open raw to snapshot schema_meta");
        schema_meta_snapshot(&raw)
    };

    let err = connection::open(&db).expect_err("a newer-comemory DB must be refused");
    assert!(
        matches!(err, comemory::errors::Error::SchemaTooNew(_)),
        "the forward-compat refusal must surface as Error::SchemaTooNew, not Error::Migration \
         (api::doctor's fallback catches exactly this variant), got: {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("0016_future"),
        "error must name the unknown key, got: {msg}"
    );
    assert!(
        msg.contains("0015_v15_console_api"),
        "error must name the highest key this build supports, got: {msg}"
    );

    let after = {
        let raw = Connection::open(&db).expect("reopen raw to snapshot schema_meta");
        schema_meta_snapshot(&raw)
    };
    assert_eq!(
        before, after,
        "a refused preflight must not write anything to schema_meta"
    );
}

#[test]
fn unknown_marker_is_refused_when_version_already_says_sixteen() {
    assert_future_marker_is_refused("16");
}

#[test]
fn unknown_marker_is_refused_when_version_still_says_fifteen_crash_before_set_version() {
    assert_future_marker_is_refused("15");
}

#[test]
fn v12_database_snapshots_to_pre_v12_bak_with_matching_pre_upgrade_contents() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    build_legacy_db(&db, 12, "12", "ffff6666");

    let conn = connection::open(&db).expect("open migrates v12 -> v13");
    assert_eq!(
        conn.query_row(
            "SELECT value FROM schema_meta WHERE key='version'",
            [],
            |r| r.get::<_, String>(0)
        )
        .expect("version"),
        migrate::CURRENT_VERSION
    );

    let bak = dir.path().join("comemory.db.pre-v12.bak");
    assert!(bak.exists(), "expected {} to exist", bak.display());
    let snap = Connection::open(&bak).expect("open snapshot as sqlite");
    assert!(
        has_memory(&snap, "ffff6666"),
        "snapshot must contain the pre-upgrade seeded row"
    );
    // The snapshot must reflect the *pre*-upgrade schema: v13's document
    // tables must not exist in it.
    let has_documents: i64 = snap
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='documents'",
            [],
            |r| r.get(0),
        )
        .expect("table probe");
    assert_eq!(has_documents, 0, "snapshot must predate the v13 migration");
}

/// A directory pre-created at the would-be `.bak` path makes `VACUUM INTO`
/// (or the validate-and-replace step ahead of it) fail deterministically —
/// not chmod, since WAL needs a writable data dir and this environment
/// runs as uid 0, where directory mode bits are ignored.
fn block_bak_with_directory(dest: &Path) {
    std::fs::create_dir_all(dest).expect("pre-create .bak path as a directory");
}

#[test]
fn destructive_upgrade_refuses_when_snapshot_is_blocked_and_leaves_db_unchanged() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    // Only 0013 (Destructive) is pending from v12.
    build_legacy_db(&db, 12, "12", "aaaa1111");

    block_bak_with_directory(&dir.path().join("comemory.db.pre-v12.bak"));

    let before = {
        let raw = Connection::open(&db).expect("open raw before failed open");
        schema_meta_snapshot(&raw)
    };

    let err = connection::open(&db)
        .expect_err("a destructive upgrade must refuse when the mandatory snapshot fails");
    assert!(
        matches!(err, comemory::errors::Error::Migration(_)),
        "a blocked mandatory snapshot must surface as Error::Migration, got: {err:?}"
    );

    let after = {
        let raw = Connection::open(&db).expect("open raw after failed open");
        schema_meta_snapshot(&raw)
    };
    assert_eq!(
        before, after,
        "a refused destructive upgrade must leave the DB at its pre-migration state"
    );
}

#[test]
fn additive_only_pending_proceeds_when_snapshot_is_blocked() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    let conn = connection::open(&db).expect("build a real v15 db");

    // Delete a single Additive migration's marker (0011_v11_memory_rank),
    // simulating "only additive work pending" — the only way to reach that
    // state at all, since 0013 (the last entry) is Destructive and so is
    // always the tail of any natural replay-to-head. This test exercises
    // preflight's own mandatory/advisory branch decision directly, rather
    // than continuing on to `migrate::run` (which would try to re-apply
    // 0011's `ALTER TABLE ADD COLUMN` against a column that already
    // exists structurally and error for an unrelated reason).
    let target = list::MIGRATIONS
        .iter()
        .find(|m| m.key == "0011_v11_memory_rank")
        .expect("0011 must exist in MIGRATIONS");
    assert_eq!(
        target.class,
        list::Class::Additive,
        "this test requires an Additive fixture migration"
    );
    conn.execute(
        "DELETE FROM schema_meta WHERE key = '0011_v11_memory_rank'",
        [],
    )
    .expect("remove the marker");

    let from = list::MIGRATIONS
        .iter()
        .position(|m| m.key == "0011_v11_memory_rank")
        .expect("index of 0011");
    let dest = dir.path().join(format!("comemory.db.pre-v{from}.bak"));
    block_bak_with_directory(&dest);

    comemory::store::migrate::preflight::preflight(&conn, &db)
        .expect("an additive-only pending migration must proceed despite a failed snapshot");
}

#[test]
fn skip_migration_backup_completes_a_destructive_upgrade_with_no_bak() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    build_legacy_db(&db, 12, "12", "bbbb2222");

    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_SKIP_MIGRATION_BACKUP", "1") };
    let result = connection::open(&db);
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_SKIP_MIGRATION_BACKUP") };

    let conn = result.expect("destructive upgrade must complete with backups skipped");
    assert_eq!(
        conn.query_row(
            "SELECT value FROM schema_meta WHERE key='version'",
            [],
            |r| r.get::<_, String>(0)
        )
        .expect("version"),
        migrate::CURRENT_VERSION
    );

    let bak_count = std::fs::read_dir(dir.path())
        .expect("read data dir")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".bak"))
        .count();
    assert_eq!(
        bak_count, 0,
        "COMEMORY_SKIP_MIGRATION_BACKUP must skip the .bak entirely"
    );
}

/// `migration_chain()` sanity check reused across this suite: it must
/// cover every entry in `MIGRATIONS` (AC-4), otherwise `build_legacy_db`
/// silently truncates its replay.
#[test]
fn migration_chain_covers_every_migration() {
    assert_eq!(migration_chain().len(), list::MIGRATIONS.len());
}
