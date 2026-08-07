#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/ingest_code.rs`. `api::ingest_code::run` is
//! called directly with a real NDJSON `&str` body — no stdin, no CLI
//! process. `cli::ingest_code::run` stays byte-compat tested against CLI
//! stdout in `tests/cli__ingest_code.rs` / `tests/cli__ingest_code_2.rs`;
//! the HTTP job route (`POST /api/v1/code/ingest`) lives in
//! `tests/serve__routes__code.rs`.

use crate::test_common::vectors;

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;
use tempfile::tempdir;

fn make_row(seed: &str, repo: &str, path: &str, blob_oid: &str) -> String {
    let embedding = vectors::vector(seed, 768);
    serde_json::to_string(&serde_json::json!({
        "repo": repo,
        "path": path,
        "blob_oid": blob_oid,
        "symbol": format!("sym_{seed}"),
        "kind": "function",
        "lang": "rust",
        "line_start": 1_u32,
        "line_end": 3_u32,
        "snippet": format!("fn {seed}() {{}}"),
        "simhash": 0_i64,
        "embedding": embedding,
    }))
    .expect("row json")
}

#[test]
fn run_inserts_rows_and_reports_files_and_rows() {
    let home = tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let row_a = make_row(
        "alpha",
        "sample",
        "src/a.rs",
        "aaaa000000000000000000000000000000000000",
    );
    let row_b = make_row(
        "beta",
        "sample",
        "src/b.rs",
        "bbbb000000000000000000000000000000000000",
    );
    let body = format!("{row_a}\n{row_b}\n");

    let resp = api::ingest_code::run(&mut ctx, &body).expect("ingest_code run");
    assert_eq!(resp.files, 2);
    assert_eq!(resp.rows, 2);

    let symbols: i64 = conn
        .query_row("SELECT count(*) FROM code_symbols", [], |r| r.get(0))
        .expect("count code_symbols");
    assert_eq!(symbols, 2);
    let vecs: i64 = conn
        .query_row("SELECT count(*) FROM code_vec", [], |r| r.get(0))
        .expect("count code_vec");
    assert_eq!(vecs, 2);
    let hits =
        comemory::store::fts::search_code(&conn, "sym_alpha", 10, None, None, (2.0, 1.0, 1.5))
            .expect("search code");
    assert_eq!(hits.len(), 1);
}

#[test]
fn run_rejects_a_chunk_row_with_a_missing_parent() {
    let home = tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let embedding = vectors::vector("orphan", 768);
    let orphan = serde_json::to_string(&serde_json::json!({
        "repo": "myrepo",
        "path": "src/big.rs",
        "blob_oid": "feed111111111111111111111111111111111111",
        "symbol": "never_seen#1",
        "kind": "function",
        "lang": "rust",
        "line_start": 1_u32,
        "line_end": 2_u32,
        "snippet": "chunk body",
        "simhash": 0_i64,
        "embedding": embedding,
        "parent_symbol": "never_seen",
        "chunk_index": 1_u32,
    }))
    .expect("row json");

    let err = api::ingest_code::run(&mut ctx, &format!("{orphan}\n"))
        .expect_err("orphan chunk row must fail");
    assert!(
        err.to_string().contains("never_seen"),
        "error must name the missing parent: {err}"
    );

    let symbols: i64 = conn
        .query_row("SELECT count(*) FROM code_symbols", [], |r| r.get(0))
        .expect("count code_symbols");
    assert_eq!(symbols, 0, "the failed batch must roll back");
}
