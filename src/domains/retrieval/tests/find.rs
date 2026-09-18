#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `retrieval::find::run` against a real store, at the api layer.
//!
//! `tests/cli__find.rs` drives the same code through the real binary and
//! covers the ranking contracts (per-domain ordering, fusion, paging,
//! `--lang` narrowing). What a subprocess cannot cleanly assert is the
//! tracking side effect, so that is what lives here: whether a run writes a
//! `retrieval_log` row is the difference between an offline evaluation and
//! one that pollutes its own training signal.

use comemory::config::{Config, Paths};
use comemory::retrieval::find;
use comemory::store::{connection, fts};
use comemory::utilities::context::Ctx;
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

/// Issue #201: a `find` page past the head writes its `retrieval_log` row but
/// bumps no access counts.
///
/// Both halves are asserted as DELTAS against the same corpus, in order, so
/// neither can hold vacuously: the offset-0 call must move
/// `memories.access_count`, proving the bump path is reachable with this
/// fixture at all, and the offset-1 call must then leave it exactly where the
/// first call left it.
///
/// Scope note: `find::track_run` bumps `code_symbols.access_count` for code
/// hits from the same `if`, but this fixture indexes no code, so asserting the
/// code column here would compare 0 to 0 and pass against any implementation.
/// The code column is covered non-vacuously by `tests/api__search_code.rs`
/// and `tests/api__context.rs` against real indexed repositories.
#[test]
fn a_deep_find_page_logs_the_query_but_bumps_no_access_counts() {
    let dir = TempDir::new().unwrap();
    let paths = paths_for(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    seed_memory(
        &conn,
        "aaaa1111",
        "frontmatter is the contract for a memory",
    );
    seed_memory(
        &conn,
        "aaaa2222",
        "frontmatter drives the rebuild of an index",
    );
    // `seed_memory` leaves `simhash` at its column default, so two rows would
    // be zero bits apart and the diversify stage would collapse them into one
    // — leaving a one-row corpus and nothing at offset 1 to assert about.
    spread_simhashes(&conn);
    let cfg = Config::defaults();

    let paged = |conn: &mut rusqlite::Connection, offset: usize| {
        let mut ctx = Ctx::borrowed(&paths, &cfg, conn);
        find::run(
            &mut ctx,
            find::Request {
                offset,
                k: Some(1),
                ..request("frontmatter", Some("memory"))
            },
            true,
        )
        .unwrap()
    };

    let head = paged(&mut conn, 0);
    assert_eq!(head.hits.len(), 1, "a one-row head page");
    assert!(head.query_id.is_some(), "the head page is logged");
    let after_head = bumped_memories(&conn);
    assert_eq!(
        after_head, 1,
        "the head page must bump its hit, else the deep-page assertion below \
         would prove nothing"
    );

    let deep = paged(&mut conn, 1);
    assert_eq!(deep.hits.len(), 1, "a one-row page at offset 1");
    assert!(
        deep.query_id.is_some(),
        "a deep page is still logged, so `comemory feedback` reaches it"
    );
    assert_eq!(log_rows(&conn), 2, "one row per run, head and deep alike");
    assert_eq!(
        bumped_memories(&conn),
        after_head,
        "a page past the head must not bump any further access counts"
    );
}

/// Give each seeded memory a far-apart SimHash so near-dup collapse cannot
/// merge the fixture corpus into a single row.
fn spread_simhashes(conn: &rusqlite::Connection) {
    conn.execute(
        "UPDATE memories SET simhash = CASE id \
           WHEN 'aaaa1111' THEN ?1 ELSE ?2 END",
        rusqlite::params![
            0x0F0F_0F0F_0F0F_0F0F_u64 as i64,
            0x7070_7070_7070_7070_u64 as i64
        ],
    )
    .unwrap();
}

/// Memories carrying any access-tracking write at all — either column.
fn bumped_memories(conn: &rusqlite::Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE access_count <> 0 OR last_accessed IS NOT NULL",
        [],
        |r| r.get(0),
    )
    .unwrap()
}
