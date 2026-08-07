//! Mirror test for `src/api/save.rs`. Calls `api::save::run` directly
//! against a `Ctx` opened on a fresh temp data-dir — proving the extracted
//! command core writes the markdown file + SQLite mirror the same way
//! `comemory save` does (`cli::save::run` is byte-compat tested against CLI
//! stdout in `tests/cli__save.rs`; the HTTP surface's AC-1 cross-check
//! lives in `tests/serve__routes__memories__write.rs`).

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::store::connection;

/// `api::save::run` with no CLI raw-vector input (HTTP-shaped call), since
/// none of these tests exercise the `--vector`/`--vector-stdin` CLI flags.
fn run(
    ctx: &mut Ctx<'_>,
    req: api::save::Request,
) -> comemory::errors::Result<api::save::Response> {
    api::save::run(ctx, req, false, None)
}

fn request(body: &str) -> api::save::Request {
    api::save::Request {
        body: body.to_string(),
        kind: Kind::Note,
        repo: "demo".to_string(),
        tags: vec!["db".to_string(), "postgres".to_string()],
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    }
}

#[test]
fn run_writes_markdown_and_sqlite_mirror() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let resp = run(&mut ctx, request("use pgbouncer in transaction mode")).expect("save run");

    assert_eq!(resp.id.len(), 8);
    assert!(resp.duplicate_of.is_none());
    assert!(resp.warnings.is_empty());
    assert!(
        std::path::Path::new(&resp.path).exists(),
        "markdown file should exist at {}",
        resp.path
    );

    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE id = ?1",
            [&resp.id],
            |r| r.get(0),
        )
        .expect("query memories row");
    assert_eq!(total, 1);
}

#[test]
fn run_rejects_self_supersede() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let body = "a memory that tries to supersede itself";
    let self_id = comemory::memory::id::memory_id(body);
    let req = api::save::Request {
        supersedes: vec![self_id],
        ..request(body)
    };

    let err = run(&mut ctx, req).expect_err("self-supersede must be rejected");
    assert!(
        err.to_string().contains("cannot supersede itself"),
        "unexpected error: {err}"
    );

    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
        .expect("query memories count");
    assert_eq!(total, 0, "a rejected save must not have written a row");
}

#[test]
fn run_rejects_quality_out_of_range() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::save::Request {
        quality: 6,
        ..request("a memory with an invalid quality rating")
    };

    let err = run(&mut ctx, req).expect_err("out-of-range quality must be rejected");
    assert!(
        err.to_string().contains("quality"),
        "unexpected error: {err}"
    );
}

#[test]
fn run_flags_a_near_duplicate() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();

    let body = "postgres advisory lock ordering fix for the migration runner";
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        run(&mut ctx, request(body)).expect("first save");
    }
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let near_dup_body = "postgres advisory lock ordering fix for the migration runners";
    let resp = run(&mut ctx, request(near_dup_body)).expect("second save");
    assert!(resp.duplicate_of.is_some(), "expected a near-dup hit");
}
