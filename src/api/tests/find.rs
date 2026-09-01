#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `api::find::run` against a real store, at the api layer.
//!
//! `tests/cli__find.rs` drives the same code through the real binary and
//! covers the ranking contracts (per-domain ordering, fusion, paging,
//! `--lang` narrowing). What a subprocess cannot cleanly assert is the
//! tracking side effect, so that is what lives here: whether a run writes a
//! `retrieval_log` row is the difference between an offline evaluation and
//! one that pollutes its own training signal.

use comemory::api::{Ctx, find};
use comemory::config::{Config, Paths};
use comemory::store::{connection, fts};
use tempfile::TempDir;

fn paths_for(dir: &TempDir) -> Paths {
    let paths = Paths::new(dir.path().to_path_buf());
    paths.ensure_dirs().unwrap();
    paths
}

/// A live memory row plus its FTS index — the lexical leg is what answers
/// here, since no vector is supplied.
fn seed_memory(conn: &rusqlite::Connection, id: &str, body: &str) {
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES(?1,'x','note','h',?2,'2026-08-01T00:00:00Z','2026-08-01T00:00:00Z','x.md')",
        rusqlite::params![id, body],
    )
    .unwrap();
    fts::index_memory(conn, id, body, "").unwrap();
}

fn request(query: &str, domain: Option<&str>) -> find::Request {
    find::Request {
        query: query.to_string(),
        k: None,
        offset: 0,
        domain: domain.map(str::to_string),
        repo: None,
        kind: None,
        lang: None,
        path: Vec::new(),
        vector: None,
        since: None,
        until: None,
        as_of: None,
    }
}

fn log_rows(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM retrieval_log", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_tracked_run_writes_exactly_one_retrieval_log_row_attributed_to_find() {
    let dir = TempDir::new().unwrap();
    let paths = paths_for(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    seed_memory(&conn, "aaaa1111", "frontmatter is the contract");
    let cfg = Config::defaults();

    let result = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        find::run(&mut ctx, request("frontmatter", None), true).unwrap()
    };

    assert_eq!(
        result.hits.len(),
        1,
        "the one seeded memory is the whole corpus"
    );
    assert_eq!(result.hits[0].id, "aaaa1111", "and it is the hit");
    assert_eq!(result.hits[0].domain, "memory");
    assert_eq!(
        result.hits[0].tier,
        Some(1),
        "a memory hit carries its lexical ladder tier — a strict match here"
    );
    let query_id = result.query_id.expect("a tracked run reports its query_id");
    assert_eq!(log_rows(&conn), 1, "one row per RUN, not one per leg");

    let source: String = conn
        .query_row(
            "SELECT source FROM retrieval_log WHERE query_id = ?1",
            [&query_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        source, "find",
        "the row is attributed to `find`, not `search`"
    );
}

#[test]
fn an_untracked_run_writes_nothing_and_reports_no_query_id() {
    let dir = TempDir::new().unwrap();
    let paths = paths_for(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    seed_memory(&conn, "aaaa1111", "frontmatter is the contract");
    let cfg = Config::defaults();

    let result = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        find::run(&mut ctx, request("frontmatter", None), false).unwrap()
    };

    assert_eq!(
        result.hits.len(),
        1,
        "results are returned either way — tracking governs side effects, not output"
    );
    assert_eq!(result.hits[0].id, "aaaa1111");
    assert!(
        result.query_id.is_none(),
        "an untracked run has no logged row to point at"
    );
    assert_eq!(
        log_rows(&conn),
        0,
        "eval and tune drive this path — a logged query there would pollute \
         the signal being measured"
    );
}

#[test]
fn an_untracked_run_does_not_bump_access_counts() {
    let dir = TempDir::new().unwrap();
    let paths = paths_for(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    seed_memory(&conn, "aaaa1111", "frontmatter is the contract");
    let cfg = Config::defaults();

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        find::run(&mut ctx, request("frontmatter", None), false).unwrap();
    }

    let accesses: i64 = conn
        .query_row(
            "SELECT access_count FROM memories WHERE id = 'aaaa1111'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        accesses, 0,
        "access_count feeds ACT-R activation, so an untracked run must leave \
         it alone or offline measurement reorders its own corpus"
    );
}

#[test]
fn an_unknown_domain_is_a_usage_error_naming_the_offender() {
    let dir = TempDir::new().unwrap();
    let paths = paths_for(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let err = find::run(&mut ctx, request("anything", Some("sideways")), false)
        .expect_err("an unknown --domain must not silently fall back to `all`");

    let msg = err.to_string();
    assert!(msg.contains("sideways"), "the error names the value: {msg}");
    assert!(
        matches!(err, comemory::errors::Error::Usage(_)),
        "a bad flag value is a usage error, not an internal one: {err:?}"
    );
}
