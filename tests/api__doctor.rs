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
}
