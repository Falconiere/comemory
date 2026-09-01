#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/doctor.rs`. Calls `api::doctor::run` directly
//! against a `Ctx::lazy` opened on a temp data-dir — proving the report
//! shape and that a writable, never-touched data dir gets its DB created
//! by `Ctx::conn` (not before) (`cli::doctor::run` is byte-compat tested
//! against CLI stdout in `tests/cli__doctor.rs`; the HTTP route lives in
//! `tests/serve__routes__maint__mod.rs`).

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::migrate::CURRENT_VERSION;

#[test]
fn run_reports_current_schema_and_embed_hint_on_a_fresh_writable_dir() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let mut cfg = Config::defaults();
    cfg.embed_hint = Some("ollama:nomic-embed-text".to_string());
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let report = api::doctor::run(&mut ctx, api::doctor::Request {}).expect("doctor run");
    assert!(report.db_writable);
    assert_eq!(report.schema_version, CURRENT_VERSION);
    assert!(report.sqlite_vec_loaded);
    assert_eq!(
        report.embed_hint.as_deref(),
        Some("ollama:nomic-embed-text")
    );
    assert_eq!(report.data_dir, home.path().to_string_lossy());

    // Console-compat: at least 10 named checks, the real memory/code vec
    // dims (1024/768), and the tokenizer registered on this fresh db.
    assert!(
        report.checks.len() >= 10,
        "expected at least 10 checks, got {}",
        report.checks.len()
    );
    assert_eq!(report.memory_vec_dim, Some(1024));
    assert_eq!(report.code_vec_dim, Some(768));
    assert!(report.tokenizer_registered);
    assert_eq!(report.markdown_files, 0);
    assert_eq!(report.mirror_drift, 0);
}

#[test]
fn run_creates_the_db_only_through_ctx_conn_on_first_use() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    assert!(!paths.db_path().exists(), "db must not exist yet");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let report = api::doctor::run(&mut ctx, api::doctor::Request {}).expect("doctor run");
    assert!(report.db_writable);
    assert!(
        paths.db_path().exists(),
        "the writable branch opens (and may create) the db via Ctx::conn"
    );
}

#[test]
fn run_reuses_an_already_migrated_db() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    // Pre-create the DB via a direct open, mirroring a prior CLI run.
    drop(comemory::store::connection::open(paths.db_path()).expect("pre-open db"));

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let report = api::doctor::run(&mut ctx, api::doctor::Request {}).expect("doctor run");
    assert!(report.db_writable);
    assert_eq!(report.schema_version, CURRENT_VERSION);
    assert!(report.embed_hint.is_none());
    assert!(
        report.unknown_migration_keys.is_empty(),
        "a real current db must report no unknown migration keys"
    );
}

/// Regression test for the fallback itself: on a database written by a
/// newer comemory, every other command is refused with
/// `Error::SchemaTooNew` (exit 70 on the CLI), but `doctor` must instead
/// fall back to a read-only probe and report the unknown key rather than
/// propagating the error — see `api::doctor`'s "Forward-compat fallback".
///
/// This fixture also exercises the "stale version reported as current"
/// case: `schema_meta.version` stays at `CURRENT_VERSION` (this DB really
/// was built by *this* build, with one extra marker bolted on to simulate
/// the newer-comemory scenario), so a naive fallback would print
/// `schema_version: CURRENT_VERSION` right next to a non-empty
/// `unknown_migration_keys` — self-contradictory, and easy to misread as
/// "up to date". The fallback must not trust that stale reading.
#[test]
fn run_falls_back_to_a_read_only_probe_and_names_the_unknown_key() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    {
        let conn = comemory::store::connection::open(paths.db_path()).expect("build a real db");
        conn.execute(
            "INSERT INTO schema_meta(key, value) VALUES('0014_future', '1')",
            [],
        )
        .expect("seed an unknown marker, simulating a newer comemory");
    }

    // A fresh Ctx, mirroring a real second invocation: the primary
    // Ctx::conn attempt must run and be refused before the fallback
    // triggers, not skip straight to the read-only path.
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let report = api::doctor::run(&mut ctx, api::doctor::Request {})
        .expect("doctor must not propagate Error::SchemaTooNew");
    assert!(report.db_writable);
    assert_eq!(
        report.unknown_migration_keys,
        vec!["0014_future".to_string()],
        "must name the exact unknown key"
    );
    assert_ne!(
        report.schema_version, CURRENT_VERSION,
        "a schema_meta.version that still matches CURRENT_VERSION despite an unknown marker \
         is stale/contradictory and must not be reported as current"
    );
    assert_eq!(
        report.schema_version, "unknown",
        "the fallback must report 'unknown' rather than a stale current-looking version"
    );
}

/// The forward-compat fallback must not swallow a genuinely broken
/// migration. Unlike an unknown *newer* marker, a KNOWN marker gone
/// missing from an otherwise fully-migrated db forces `migrate::run` to
/// re-apply that migration's SQL — whose `CREATE TABLE` statements are not
/// idempotent against a schema that already has them, so it fails outright.
/// That must propagate as `Error::Migration`, not be silently reported as a
/// clean bill of health the way `Error::SchemaTooNew` is — the exact bug
/// this regression test exists to catch: `doctor --json` reporting
/// `unknown_migration_keys: []` on a database `comemory list` itself
/// refuses to open.
#[test]
fn run_propagates_a_genuinely_broken_migration_rather_than_falling_back() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    {
        let conn = comemory::store::connection::open(paths.db_path()).expect("build a real db");
        conn.execute(
            "DELETE FROM schema_meta WHERE key = '0013_v13_documents'",
            [],
        )
        .expect("remove an already-applied marker, forcing a broken re-apply");
    }

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let err = api::doctor::run(&mut ctx, api::doctor::Request {})
        .expect_err("a genuinely broken migration must propagate, not fall back");
    assert!(
        matches!(err, comemory::errors::Error::Migration(_)),
        "expected Error::Migration, got: {err}"
    );
}
