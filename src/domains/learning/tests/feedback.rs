#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Atomicity and concurrency tests for the combined feedback command core.

use std::sync::{Arc, Barrier};
use std::thread;

use comemory::config::{Config, Paths};
use comemory::domains::learning::feedback::{self, Request};
use comemory::errors::Error;
use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::connection;
use comemory::utilities::context::Ctx;

use crate::test_common as common;

fn request(query_id: &str, memory_id: &str, code_id: i64) -> Request {
    Request {
        query_id: query_id.into(),
        used: vec![memory_id.into()],
        irrelevant: Vec::new(),
        used_code: vec![code_id.to_string()],
        irrelevant_code: Vec::new(),
        source: None,
    }
}

fn seed_symbol(paths: &Paths) -> i64 {
    paths.ensure_dirs().expect("ensure data dirs");
    let conn = connection::open(paths.db_path()).expect("open");
    code_row::insert(
        &conn,
        &CodeSymbolRow {
            repo: "demo",
            path: "src/lib.rs",
            blob_oid: "oid",
            symbol: "alpha",
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 2,
            snippet: "fn alpha() {}",
            simhash: 0,
            parent_id: None,
        },
    )
    .expect("seed code symbol")
}

fn counts(paths: &Paths) -> (i64, i64, i64) {
    let conn = connection::open(paths.db_path()).expect("open counts");
    conn.query_row(
        "SELECT (SELECT count(*) FROM feedback), \
                (SELECT count(*) FROM code_feedback), \
                (SELECT count(*) FROM feedback_events)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .expect("read counts")
}

#[test]
fn mixed_feedback_rolls_back_memory_rows_when_code_identity_is_missing() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let err = feedback::run(
        &mut ctx,
        request("q-20260920-aabbcc01", "aaaaaaa1", 999_999),
    )
    .expect_err("missing code identity must fail the mixed request");
    assert!(
        matches!(&err, Error::Config(message) if message ==
            "code feedback: symbol id 999999 not found in code_symbols \
             (re-indexed away or never existed); re-run comemory search-code for current ids"),
        "unexpected error: {err}"
    );
    assert_eq!(
        counts(&paths),
        (0, 0, 0),
        "memory and code counters/events must share one rollback boundary"
    );
}

#[test]
fn malformed_memory_id_refuses_mixed_feedback_before_writing_code() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    let code_id = seed_symbol(&paths);
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let error = feedback::run(
        &mut ctx,
        request("q-20260920-aabbcc05", "not-an-id", code_id),
    )
    .expect_err("malformed memory id must refuse the entire mixed request");
    assert!(
        matches!(&error, Error::Config(message) if message ==
            "--used: invalid memory id `not-an-id` (expected 8 lowercase hex chars)"),
        "unexpected error: {error}"
    );
    assert_eq!(counts(&paths), (0, 0, 0));
}

#[test]
fn concurrent_mixed_feedback_serializes_without_lost_or_duplicate_counts() {
    let sb = common::runner::Sandbox::new();
    let data_dir = sb.data_dir().clone();
    let paths = Paths::new(&data_dir);
    let code_id = seed_symbol(&paths);
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();

    for (query_id, memory_id) in [
        ("q-20260920-aabbcc02", "aaaaaaa2"),
        ("q-20260920-aabbcc03", "aaaaaaa2"),
    ] {
        let thread_dir = data_dir.clone();
        let thread_barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let paths = Paths::new(thread_dir);
            let cfg = Config::defaults();
            let mut ctx = Ctx::lazy(&paths, &cfg);
            thread_barrier.wait();
            feedback::run(&mut ctx, request(query_id, memory_id, code_id))
        }));
    }
    barrier.wait();
    for handle in handles {
        handle
            .join()
            .expect("feedback thread panicked")
            .expect("concurrent feedback failed");
    }

    let conn = connection::open(paths.db_path()).expect("open result");
    let memory_uses: i64 = conn
        .query_row(
            "SELECT used_count FROM feedback WHERE memory_id = 'aaaaaaa2'",
            [],
            |row| row.get(0),
        )
        .expect("memory counter");
    let code_uses: i64 = conn
        .query_row(
            "SELECT used_count FROM code_feedback \
              WHERE repo = 'demo' AND path = 'src/lib.rs' AND symbol = 'alpha'",
            [],
            |row| row.get(0),
        )
        .expect("code counter");
    assert_eq!((memory_uses, code_uses), (2, 2));
    assert_eq!(counts(&paths).2, 4, "two mixed calls write four events");
}
