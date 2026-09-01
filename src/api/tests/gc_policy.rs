#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/gc_policy.rs` — `GET|PUT /api/v1/gc/policy`
//! (console-api spec §9, AC-17's gc half).
//!
//! Everything is real: a real temp data-dir, a real `config.toml` written
//! by the shared `patch_config_file` primitive and read back through the
//! layered `Config` loader, a real `gc_runs` row, and a real trashed file
//! whose mtime is pushed two days back so `api::gc::run` under the patched
//! window actually reaps it.

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::{connection, gc_runs};
use tempfile::TempDir;

/// Load `config.toml` through the same layered path a fresh process uses,
/// so a test asserts on what the next run would really see.
fn reload(paths: &Paths) -> Config {
    comemory::cli::load_config(paths).expect("reload config")
}

/// Put a real file in `memories/.trash/` and back-date its mtime by `days`.
fn trash_file(paths: &Paths, name: &str, days: u64) -> std::path::PathBuf {
    let dir = paths.trash_dir();
    std::fs::create_dir_all(&dir).expect("create trash dir");
    let path = dir.join(name);
    std::fs::write(&path, "---\nid: deadbeef\n---\nbody\n").expect("write trashed file");
    let file = std::fs::File::options()
        .write(true)
        .open(&path)
        .expect("open trashed file");
    let when = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 86_400);
    file.set_modified(when).expect("back-date mtime");
    path
}

#[test]
fn get_reports_the_configured_windows_without_creating_the_db() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    assert!(!paths.db_path().exists(), "db must not exist yet");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let policy = api::gc_policy::get(&mut ctx).expect("gc policy get");

    assert_eq!(policy.trash_retention_days, cfg.prune.trash_retention_days);
    assert_eq!(
        policy.telemetry_retention_days,
        cfg.prune.learning_retention_days
    );
    assert!(policy.last_run.is_none());
    assert!(policy.last_run_at.is_none());
    assert!(
        !paths.db_path().exists(),
        "a policy read must not create comemory.db"
    );
}

#[test]
fn get_reports_the_newest_recorded_run() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    gc_runs::insert(
        &conn,
        "1111111111111111",
        "2026-08-01T00:00:00Z",
        1,
        2,
        3,
        4,
    )
    .expect("insert older run");
    gc_runs::insert(
        &conn,
        "2222222222222222",
        "2026-08-30T12:00:00Z",
        7,
        70,
        17,
        2048,
    )
    .expect("insert newer run");
    drop(conn);

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let policy = api::gc_policy::get(&mut ctx).expect("gc policy get");

    let last = policy.last_run.expect("a run was recorded");
    assert_eq!(last.id, "2222222222222222");
    assert_eq!(last.removed, 7);
    assert_eq!(last.bytes_freed, 2048);
    assert_eq!(policy.last_run_at.as_deref(), Some("2026-08-30T12:00:00Z"));
}

#[test]
fn update_persists_both_windows_and_leaves_other_keys_alone() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    std::fs::write(paths.config_file(), "[prune]\nmin_feedback = 0.4\n").expect("seed config.toml");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let policy = api::gc_policy::update(
        &mut ctx,
        api::gc_policy::UpdateRequest {
            trash_retention_days: Some(3),
            telemetry_retention_days: Some(45),
        },
    )
    .expect("gc policy update");

    assert_eq!(policy.trash_retention_days, 3);
    assert_eq!(policy.telemetry_retention_days, 45);

    let reloaded = reload(&paths);
    assert_eq!(reloaded.prune.trash_retention_days, 3);
    assert_eq!(reloaded.prune.learning_retention_days, 45);
    assert_eq!(
        reloaded.prune.min_feedback, 0.4,
        "an unrelated [prune] key must survive the patch"
    );
}

#[test]
fn update_patches_only_the_supplied_key() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    api::gc_policy::update(
        &mut ctx,
        api::gc_policy::UpdateRequest {
            trash_retention_days: Some(2),
            telemetry_retention_days: None,
        },
    )
    .expect("gc policy update");

    let raw = std::fs::read_to_string(paths.config_file()).expect("read config.toml");
    assert!(raw.contains("trash_retention_days = 2"), "config: {raw}");
    assert!(
        !raw.contains("learning_retention_days"),
        "an unsupplied key must not be written: {raw}"
    );
}

#[test]
fn update_rejects_a_zero_window_and_leaves_the_file_untouched() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    std::fs::write(paths.config_file(), "[prune]\ntrash_retention_days = 30\n")
        .expect("seed config.toml");
    let before = std::fs::read(paths.config_file()).expect("read before");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let err = api::gc_policy::update(
        &mut ctx,
        api::gc_policy::UpdateRequest {
            trash_retention_days: Some(0),
            telemetry_retention_days: None,
        },
    )
    .expect_err("a zero window must be refused");

    assert!(
        matches!(err, comemory::errors::Error::BadRequest(_)),
        "expected Error::BadRequest, got {err:?}"
    );
    let after = std::fs::read(paths.config_file()).expect("read after");
    assert_eq!(before, after, "a refused update must not touch the file");
}

/// AC-17 (gc half): a one-day trash window makes `gc` reap a two-day-old
/// trashed file that the 30-day default would have kept.
#[test]
fn update_to_one_day_makes_gc_reap_a_two_day_old_trashed_file() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    connection::open(paths.db_path()).expect("create db");
    let trashed = trash_file(&paths, "deadbeef-old-memory.md", 2);

    // Under the shipped 30-day window the file survives.
    let defaults = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &defaults);
    let resp = api::gc::run(&mut ctx, api::gc::Request {}).expect("gc under defaults");
    assert_eq!(resp.removed, 0);
    assert!(trashed.exists(), "the default window keeps it");

    // Patch the window to one day, reload the way a server does, sweep again.
    let mut ctx = Ctx::lazy(&paths, &defaults);
    let policy = api::gc_policy::update(
        &mut ctx,
        api::gc_policy::UpdateRequest {
            trash_retention_days: Some(1),
            telemetry_retention_days: None,
        },
    )
    .expect("gc policy update");
    assert_eq!(policy.trash_retention_days, 1);

    let patched = reload(&paths);
    assert_eq!(patched.prune.trash_retention_days, 1);
    let mut ctx = Ctx::lazy(&paths, &patched);
    let resp = api::gc::run(&mut ctx, api::gc::Request {}).expect("gc under the new window");
    assert_eq!(resp.removed, 1, "the two-day-old file must be reaped");
    assert!(resp.bytes_freed > 0);
    assert!(!trashed.exists());

    // And the sweep is now the policy's `last_run`.
    let mut ctx = Ctx::lazy(&paths, &patched);
    let policy = api::gc_policy::get(&mut ctx).expect("gc policy get");
    assert_eq!(policy.last_run.map(|r| r.removed), Some(1));
    assert!(policy.last_run_at.is_some());
}
