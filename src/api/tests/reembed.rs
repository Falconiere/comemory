#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/reembed.rs` — `POST /api/v1/doctor/reembed`
//! (console-api spec §8, AC-16's core half).
//!
//! Real data end to end: memories saved through `api::save` into a real
//! temp data-dir, and a REAL shell script standing in for the embedder —
//! written to disk, made executable, and run by `embed::embed_query`
//! exactly as an operator's `--embed-cmd` would be. There is no mock: the
//! thing under test is the interaction between the shell-out, the vec0
//! dim guard, and the per-row transaction, and a mock would hide all three.

use comemory::api::index_code::ProgressSink;
use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::store::connection;
use std::sync::atomic::{AtomicU64, Ordering};
use tempfile::TempDir;

/// Write an executable script emitting a `dim`-wide constant embedding.
fn embed_script(dir: &TempDir, name: &str, dim: usize) -> String {
    let values = vec!["0.01"; dim].join(",");
    write_script(
        dir,
        name,
        &format!("cat >/dev/null\nprintf '{{\"embedding\":[{values}]}}'\n"),
    )
}

/// Write `body` as an executable `sh` script under `dir` and return its path.
fn write_script(dir: &TempDir, name: &str, body: &str) -> String {
    let path = dir.path().join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");
    }
    path.to_string_lossy().into_owned()
}

/// Save `count` memories through the real command core.
fn seed(paths: &Paths, count: usize) {
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    for i in 0..count {
        let mut ctx = Ctx::borrowed(paths, &cfg, &mut conn);
        let req = api::save::Request {
            body: format!("re-embed subject number {i}: postgres advisory locks"),
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
}

/// `SELECT COUNT(*)` over `table`.
fn count(paths: &Paths, table: &str) -> i64 {
    let conn = connection::open(paths.db_path()).expect("open db");
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .expect("count rows")
}

/// A sink that counts progress calls and can report itself cancelled after
/// a fixed number of rows — the real `ProgressSink` contract, driven from a
/// test instead of from the job registry.
struct CountingSink {
    progress: AtomicU64,
    logs: AtomicU64,
    cancel_after: Option<u64>,
}

impl CountingSink {
    fn new(cancel_after: Option<u64>) -> Self {
        Self {
            progress: AtomicU64::new(0),
            logs: AtomicU64::new(0),
            cancel_after,
        }
    }
}

impl ProgressSink for CountingSink {
    fn on_progress(&self, done: u64, _total: u64) {
        self.progress.store(done, Ordering::SeqCst);
    }

    fn on_log(&self, _line: &str) {
        self.logs.fetch_add(1, Ordering::SeqCst);
    }

    fn is_cancelled(&self) -> bool {
        self.cancel_after
            .is_some_and(|n| self.progress.load(Ordering::SeqCst) >= n)
    }
}

#[test]
fn run_writes_one_memory_vec_row_per_live_memory_ac16() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    seed(&paths, 3);
    assert_eq!(count(&paths, "memory_vec"), 0, "saves were lexical-only");

    let cmd = embed_script(&home, "embed-1024.sh", 1024);
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let sink = CountingSink::new(None);
    let resp = api::reembed::run(
        &mut ctx,
        api::reembed::Request {
            target: api::reembed::Target::Memories,
            batch: None,
        },
        &cmd,
        Some(&sink),
    )
    .expect("reembed run");

    assert_eq!(resp.memories, 3);
    assert_eq!(resp.code, 0);
    assert_eq!(resp.failed, 0);
    assert_eq!(resp.skipped, 0);
    assert_eq!(count(&paths, "memory_vec"), 3);
    assert_eq!(sink.progress.load(Ordering::SeqCst), 3);
}

#[test]
fn run_is_idempotent_and_replaces_rather_than_duplicates() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    seed(&paths, 2);
    let cmd = embed_script(&home, "embed-1024.sh", 1024);
    let cfg = Config::defaults();

    for _ in 0..2 {
        let mut ctx = Ctx::lazy(&paths, &cfg);
        let resp = api::reembed::run(
            &mut ctx,
            api::reembed::Request {
                target: api::reembed::Target::Memories,
                batch: Some(8),
            },
            &cmd,
            None,
        )
        .expect("reembed run");
        assert_eq!(resp.memories, 2);
    }
    assert_eq!(
        count(&paths, "memory_vec"),
        2,
        "a second run must replace, not duplicate"
    );
}

#[test]
fn run_fails_with_embedder_when_the_first_row_cannot_be_embedded() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    seed(&paths, 2);

    let cmd = write_script(&home, "embed-broken.sh", "cat >/dev/null\nexit 3\n");
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let err = api::reembed::run(
        &mut ctx,
        api::reembed::Request {
            target: api::reembed::Target::Memories,
            batch: None,
        },
        &cmd,
        None,
    )
    .expect_err("a broken embedder must fail the run");

    assert!(
        matches!(err, comemory::errors::Error::Embedder(_)),
        "expected Error::Embedder, got {err:?}"
    );
    assert_eq!(count(&paths, "memory_vec"), 0);
}

#[test]
fn run_counts_a_later_row_failure_and_keeps_the_rows_already_written() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    seed(&paths, 3);

    // A real flaky embedder: it counts its own invocations in a file and
    // fails from the third one onward. Call 1 is the width probe, call 2
    // is the first row (which must succeed — a first-row failure is a
    // whole-run `Error::Embedder`), calls 3 and 4 are the two failures.
    let counter = home.path().join("call-count");
    let values = vec!["0.01"; 1024].join(",");
    let cmd = write_script(
        &home,
        "embed-flaky.sh",
        &format!(
            "cat >/dev/null\nn=$(cat '{counter}' 2>/dev/null || echo 0)\nn=$((n + 1))\n\
             printf '%s' \"$n\" > '{counter}'\nif [ \"$n\" -gt 2 ]; then exit 4; fi\n\
             printf '{{\"embedding\":[{values}]}}'\n",
            counter = counter.display()
        ),
    );

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = api::reembed::run(
        &mut ctx,
        api::reembed::Request {
            target: api::reembed::Target::Memories,
            batch: None,
        },
        &cmd,
        None,
    )
    .expect("a flaky embedder must not fail the whole run");

    assert_eq!(resp.memories, 1, "the one successful row is written");
    assert_eq!(resp.failed, 2, "the two later failures are counted");
    assert_eq!(count(&paths, "memory_vec"), 1);
}

#[test]
fn run_rejects_a_wrong_width_vector_without_writing_anything() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    seed(&paths, 2);

    let cmd = embed_script(&home, "embed-16.sh", 16);
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let err = api::reembed::run(
        &mut ctx,
        api::reembed::Request {
            target: api::reembed::Target::Memories,
            batch: None,
        },
        &cmd,
        None,
    )
    .expect_err("a dim mismatch must fail the run");

    assert!(
        matches!(
            err,
            comemory::errors::Error::VecDimMismatch {
                expected: 1024,
                got: 16
            }
        ),
        "expected a 1024/16 dim mismatch, got {err:?}"
    );
    assert_eq!(count(&paths, "memory_vec"), 0);
}

#[test]
fn run_stops_at_the_next_row_when_the_sink_reports_cancelled() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    seed(&paths, 4);

    let cmd = embed_script(&home, "embed-1024.sh", 1024);
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let sink = CountingSink::new(Some(2));
    let err = api::reembed::run(
        &mut ctx,
        api::reembed::Request {
            target: api::reembed::Target::Memories,
            batch: None,
        },
        &cmd,
        Some(&sink),
    )
    .expect_err("a cancelled run reports Error::Cancelled");

    assert!(
        matches!(err, comemory::errors::Error::Cancelled),
        "expected Error::Cancelled, got {err:?}"
    );
    assert_eq!(
        count(&paths, "memory_vec"),
        2,
        "rows written before the cancel stay written"
    );
}

#[test]
fn run_over_an_empty_corpus_is_a_no_op() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    connection::open(paths.db_path()).expect("create db");

    // A script that would fail if it were ever called — nothing to embed.
    let cmd = write_script(&home, "embed-never.sh", "exit 9\n");
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = api::reembed::run(&mut ctx, api::reembed::Request::default(), &cmd, None)
        .expect("empty reembed run");

    assert_eq!(resp, api::reembed::Response::default());
}

/// Insert one parent `code_symbols` row through the real writer, so the
/// code leg has something to re-embed.
fn seed_code(paths: &Paths, symbol: &str) {
    let conn = connection::open(paths.db_path()).expect("open db");
    let snippet = format!("fn {symbol}() {{ acquire_advisory_lock(); }}");
    comemory::store::code_row::insert(
        &conn,
        &comemory::store::code_row::CodeSymbolRow {
            repo: "demo",
            path: "src/lock.rs",
            blob_oid: "0123456789abcdef0123456789abcdef01234567",
            symbol,
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 3,
            snippet: &snippet,
            simhash: comemory::simhash::simhash64(comemory::simhash::tokens(&snippet)) as i64,
            parent_id: None,
        },
    )
    .expect("seed code symbol");
}

#[test]
fn a_both_run_embeds_the_leg_the_width_fits_and_reports_the_other_skipped() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    seed(&paths, 2);
    seed_code(&paths, "acquire");

    // 1024 is `memory_vec`'s width; `code_vec` is 768, so the code leg
    // cannot run and must be reported rather than attempted.
    let cmd = embed_script(&home, "embed-1024.sh", 1024);
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let sink = CountingSink::new(None);
    let resp = api::reembed::run(
        &mut ctx,
        api::reembed::Request {
            target: api::reembed::Target::Both,
            batch: None,
        },
        &cmd,
        Some(&sink),
    )
    .expect("a both run must not fail on the leg that does not fit");

    assert_eq!(resp.memories, 2);
    assert_eq!(resp.code, 0);
    assert_eq!(resp.failed, 0);
    assert_eq!(resp.skipped_legs, vec!["code"]);
    assert_eq!(count(&paths, "memory_vec"), 2);
    assert_eq!(count(&paths, "code_vec"), 0, "the code leg never ran");
}

#[test]
fn an_explicit_code_leg_the_width_does_not_fit_is_refused_before_any_write() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    seed(&paths, 1);
    seed_code(&paths, "release");

    let cmd = embed_script(&home, "embed-1024.sh", 1024);
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let err = api::reembed::run(
        &mut ctx,
        api::reembed::Request {
            target: api::reembed::Target::Code,
            batch: None,
        },
        &cmd,
        None,
    )
    .expect_err("an explicit leg that does not fit is an error, not a skip");

    assert!(
        matches!(
            err,
            comemory::errors::Error::VecDimMismatch {
                expected: 768,
                got: 1024
            }
        ),
        "expected a 768/1024 dim mismatch, got {err:?}"
    );
    assert_eq!(count(&paths, "code_vec"), 0);
    assert_eq!(count(&paths, "memory_vec"), 0, "the other leg is untouched");
}

#[test]
fn a_both_run_whose_width_fits_neither_leg_is_a_bad_request() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    seed(&paths, 1);
    seed_code(&paths, "probe_target");

    let cmd = embed_script(&home, "embed-16.sh", 16);
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let err = api::reembed::run(
        &mut ctx,
        api::reembed::Request {
            target: api::reembed::Target::Both,
            batch: None,
        },
        &cmd,
        None,
    )
    .expect_err("no runnable leg is a BadRequest");

    match err {
        comemory::errors::Error::BadRequest(msg) => {
            assert!(msg.contains("16-dim"), "the probed width is named: {msg}");
            assert!(msg.contains("memory_vec is 1024-dim"), "{msg}");
            assert!(msg.contains("code_vec is 768-dim"), "{msg}");
        }
        other => panic!("expected BadRequest, got {other:?}"),
    }
    assert_eq!(count(&paths, "memory_vec"), 0);
    assert_eq!(count(&paths, "code_vec"), 0);
}

#[test]
fn an_empty_leg_is_never_named_as_a_width_mismatch() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    // Code only — no memories at all. `memory_vec` is 1024-dim, which the
    // embedder matches exactly; it simply has nothing to embed, so the
    // failure must be about the code leg alone.
    seed_code(&paths, "only_code_here");

    let cmd = embed_script(&home, "embed-1024.sh", 1024);
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let err = api::reembed::run(
        &mut ctx,
        api::reembed::Request {
            target: api::reembed::Target::Both,
            batch: None,
        },
        &cmd,
        None,
    )
    .expect_err("the only leg with rows does not fit");

    match err {
        comemory::errors::Error::BadRequest(msg) => {
            assert!(msg.contains("code_vec is 768-dim"), "{msg}");
            assert!(
                !msg.contains("memory_vec"),
                "an empty leg must not be reported as a mismatch: {msg}"
            );
        }
        other => panic!("expected BadRequest, got {other:?}"),
    }
    assert_eq!(count(&paths, "code_vec"), 0);
}
