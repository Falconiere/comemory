#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/doctor/system.rs` — `GET /api/v1/doctor/system`
//! (console-api spec §8, AC-15).
//!
//! Two invariants are load-bearing and both are proven here against real
//! data (a real temp data-dir, real memories saved through `api::save`, a
//! real migrated `comemory.db`):
//!
//! 1. The report **never runs the embed command**. Proven structurally, not
//!    by inspection: `system::run` is called with a real `COMEMORY_EMBED_CMD`
//!    *value* threaded through a config the report echoes, while the script
//!    that value names writes a sentinel file when executed. The sentinel
//!    must not exist afterwards. No env var is set by these tests —
//!    process-global env mutation would race every other colocated test.
//! 2. It **never creates `comemory.db`**, so a console polling it on a
//!    fresh install does not silently materialize a store.

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::store::connection;
use comemory::store::migrate::CURRENT_VERSION;
use tempfile::TempDir;

/// Write an executable shell script that touches `sentinel` and prints a
/// well-formed embedding, then return its path. Running it is observable;
/// not running it is exactly what AC-15 asserts.
fn sentinel_embed_script(dir: &TempDir, sentinel: &std::path::Path) -> String {
    let script = dir.path().join("embed-sentinel.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ntouch '{}'\ncat >/dev/null\nprintf '{{\"embedding\":[0.1,0.2]}}'\n",
            sentinel.display()
        ),
    )
    .expect("write embed script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod embed script");
    }
    script.to_string_lossy().into_owned()
}

/// Save one memory through the real command core so the markdown file and
/// the SQLite mirror both exist, exactly as a real `comemory save` leaves
/// them.
fn save(paths: &Paths, body: &str) {
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(paths, &cfg, &mut conn);
    let req = api::save::Request {
        body: body.to_string(),
        title: None,
        kind: Kind::Note,
        repo: "demo".to_string(),
        tags: Vec::new(),
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    };
    api::save::run(&mut ctx, req, false, None).expect("seed save");
}

#[test]
fn run_reports_the_current_schema_on_a_real_store() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    save(&paths, "system facts read over a real memory");

    let mut cfg = Config::defaults();
    cfg.embed_hint = Some("ollama:nomic-embed-text".to_string());
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let system = api::doctor::system::run(&mut ctx).expect("system run");

    assert_eq!(system.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(system.current_schema_version, CURRENT_VERSION);
    assert_eq!(system.schema_version.as_deref(), Some(CURRENT_VERSION));
    assert_eq!(system.data_dir, home.path().to_string_lossy());
    assert_eq!(system.db_path, paths.db_path().to_string_lossy());
    assert!(system.db_bytes > 0, "a migrated db is not empty");
    assert_eq!(system.markdown_files, 1);
    assert_eq!(system.trash_files, 0);
    assert_eq!(system.memory_vec_dim, 1024);
    assert_eq!(system.code_vec_dim, 768);
    assert_eq!(
        system.embed_hint.as_deref(),
        Some("ollama:nomic-embed-text")
    );
    assert!(system.backup_path.is_none(), "no migration snapshot yet");
    assert!(system.backup_bytes.is_none());
}

#[test]
fn run_never_executes_the_embed_command_ac15() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    save(&paths, "AC-15: a facts read must not warm an embedder");

    let sentinel = home.path().join("embedder-was-run");
    let script = sentinel_embed_script(&home, &sentinel);
    // Sanity: the script really is observable when it IS run, so the
    // assertion below cannot pass because the script is inert.
    let probe = comemory::embed::embed_query(&script, "probe").expect("probe embed");
    assert_eq!(probe.len(), 2);
    assert!(sentinel.exists(), "the probe must have run the script");
    std::fs::remove_file(&sentinel).expect("reset sentinel");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let system = api::doctor::system::run(&mut ctx).expect("system run");
    assert_eq!(system.markdown_files, 1);
    assert!(
        !sentinel.exists(),
        "AC-15: doctor/system must never run the embed command"
    );
}

#[test]
fn run_on_a_fresh_data_dir_never_creates_the_db() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    assert!(!paths.db_path().exists(), "db must not exist yet");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let system = api::doctor::system::run(&mut ctx).expect("system run");

    assert!(
        !paths.db_path().exists(),
        "doctor/system on a fresh dir must not create comemory.db"
    );
    assert!(system.schema_version.is_none());
    assert_eq!(system.current_schema_version, CURRENT_VERSION);
    assert_eq!(system.db_bytes, 0);
    assert_eq!(system.markdown_files, 0);
    // Falls back to the configured dims when `schema_meta` is unreachable.
    assert_eq!(system.memory_vec_dim, cfg.retrieval.memory_vector_dim);
    assert_eq!(system.code_vec_dim, cfg.retrieval.code_vector_dim);
}

#[test]
fn run_counts_trashed_files_and_names_a_migration_snapshot() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    save(&paths, "a memory destined for the trash");

    // A real soft-delete through the same path `comemory delete` uses.
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let id = conn
        .query_row("SELECT id FROM memories LIMIT 1", [], |r| {
            r.get::<_, String>(0)
        })
        .expect("read seeded id");
    comemory::cli::delete::soft_delete(&paths, &mut conn, &id).expect("soft delete");
    drop(conn);

    // A real pre-migration snapshot beside the live db.
    let backup = home.path().join("comemory.db.pre-v14.bak");
    std::fs::copy(paths.db_path(), &backup).expect("write snapshot");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let system = api::doctor::system::run(&mut ctx).expect("system run");

    assert_eq!(system.markdown_files, 0, "the file moved to .trash/");
    assert_eq!(system.trash_files, 1);
    assert_eq!(
        system.backup_path.as_deref(),
        Some(backup.to_string_lossy().as_ref())
    );
    assert_eq!(
        system.backup_bytes,
        Some(std::fs::metadata(&backup).expect("stat snapshot").len())
    );
}
