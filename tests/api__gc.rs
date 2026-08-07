//! Mirror test for `src/api/gc.rs`. Calls `api::gc::run` directly against a
//! `Ctx::lazy` opened on a temp data-dir, proving the trash sweep,
//! telemetry retention purge, and — critically — that a fresh data dir with
//! no prior `comemory.db` is never touched by `run` (`cli::gc::run` is
//! byte-compat tested against CLI stdout in `tests/cli__gc.rs`; the HTTP
//! route lives in `tests/serve__routes__maint__prune.rs`).

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

    let conn = rusqlite::Connection::open_with_flags(
        db_path(&home),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("reopen db read-only");
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM retrieval_log", [], |r| r.get(0))
        .expect("count retrieval_log");
    assert_eq!(remaining, 1, "fresh row must survive");
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
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(31 * 86_400))
        .expect("backdate mtime");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = api::gc::run(&mut ctx, api::gc::Request {}).expect("gc run");

    assert_eq!(resp.removed, 1);
    assert!(!old.exists(), "old trash entry must be deleted");
    assert!(fresh.exists(), "fresh trash entry must be kept");
}
