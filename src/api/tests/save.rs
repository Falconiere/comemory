#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
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
        title: None,
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

/// A save that carries `--vector` writes exactly one `memory_vec` row, and
/// a re-save of the same id (a new vector, matching `id::memory_id`'s
/// content-hash unaffected fields) replaces rather than duplicates it — the
/// `store::vector::replace_memory` path `write_sqlite_mirror` now calls.
#[test]
fn run_with_a_vector_writes_one_row_and_a_resave_replaces_it() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let dim = comemory::store::vector::dim_memory(&conn).expect("dim");

    let body = "use a fixed advisory lock key derived from the table name";
    let first_vec = vec![0.25_f32; dim];
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let req = api::save::Request {
            vector: Some(first_vec),
            ..request(body)
        };
        run(&mut ctx, req).expect("first save with vector");
    }
    let memory_id = comemory::memory::id::memory_id(body);
    let count_rows = |conn: &rusqlite::Connection| -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM memory_vec WHERE memory_id = ?1",
            [&memory_id],
            |r| r.get(0),
        )
        .expect("count memory_vec")
    };
    assert_eq!(count_rows(&conn), 1);

    // Re-save the SAME body (a lexical-only re-save carries no vector by
    // default; here the caller supplies a different one, e.g. a re-embed by
    // hand) — same content hash, same memory id.
    let second_vec = vec![0.75_f32; dim];
    let expected_blob = comemory::store::embed::to_vec_blob(&second_vec);
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let req = api::save::Request {
            vector: Some(second_vec),
            ..request(body)
        };
        run(&mut ctx, req).expect("second save with vector");
    }
    assert_eq!(
        count_rows(&conn),
        1,
        "a re-save must replace the memory_vec row, not duplicate it"
    );

    // Row COUNT alone cannot tell a replace from a no-op that left the first
    // vector in place. Assert the stored bytes are the SECOND vector's.
    let stored: Vec<u8> = conn
        .query_row(
            "SELECT embedding FROM memory_vec WHERE memory_id = ?1",
            [&memory_id],
            |r| r.get(0),
        )
        .expect("read stored embedding");
    assert_eq!(
        stored, expected_blob,
        "the re-saved vector must overwrite the first, not be discarded"
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
