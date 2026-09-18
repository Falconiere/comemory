#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/retrieval/context.rs`. Seeds a real memory via the
//! `comemory` binary, then calls `retrieval::context::run` directly against a
//! `Ctx` opened on the same data-dir — proving the extracted command core
//! assembles the same bundle `comemory context` does (`cli::context::run`
//! is byte-compat tested against CLI stdout in `tests/cli__context.rs`).

use assert_cmd::Command;
use comemory::config::{Config, Paths};
use comemory::retrieval;
use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::connection;
use comemory::utilities::context::Ctx;

fn save(home: &tempfile::TempDir, body: &str) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["save", body, "--kind", "decision", "--repo", "demo"])
        .assert()
        .success();
}

fn request(query: &str) -> retrieval::context::Request {
    retrieval::context::Request {
        query: query.to_string(),
        k: None,
        offset: 0,
        repo: None,
        vector: None,
        since: None,
        until: None,
        as_of: None,
    }
}

#[test]
fn run_assembles_a_bundle_for_the_matched_memory() {
    let home = tempfile::tempdir().expect("tempdir");
    save(&home, "run_migration applies pending schema changes");
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let result =
        retrieval::context::run(&mut ctx, request("run_migration"), false).expect("context run");
    assert_eq!(result.bundle.query, "run_migration");
    assert_eq!(result.bundle.memories.len(), 1);
    assert!(result.bundle.memories[0].body.contains("run_migration"));
}

/// Issue #152, API twin: the bundle's memory `score` is the pipeline's
/// `final_score` — the same number `retrieval::search::run` ranks the same query
/// with — never the old `0.0` placeholder.
#[test]
fn run_carries_the_search_pipeline_score_onto_each_memory_row() {
    let home = tempfile::tempdir().expect("tempdir");
    save(
        &home,
        "run_migration applies pending schema changes in order",
    );
    save(
        &home,
        "schema changes land through run_migration and a backup",
    );
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();

    let search = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let req = retrieval::search::Request {
            query: "run_migration".to_string(),
            k: None,
            offset: 0,
            repo: None,
            kind: None,
            vector: None,
            since: None,
            until: None,
            as_of: None,
        };
        retrieval::search::run(&mut ctx, req, false).expect("search run")
    };
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let result =
        retrieval::context::run(&mut ctx, request("run_migration"), false).expect("context run");

    let expected: Vec<(&str, f64)> = search
        .hits
        .iter()
        .map(|h| (h.memory_id.as_str(), h.parts.final_score))
        .collect();
    let got: Vec<(&str, f64)> = result
        .bundle
        .memories
        .iter()
        .map(|m| (m.id.as_str(), m.score))
        .collect();
    assert_eq!(got.len(), 2, "both memories match: {got:?}");
    assert!(
        got.iter().all(|(_, s)| *s > 0.0),
        "no placeholder scores: {got:?}"
    );
    // Same ids in the same order; the activation prior reads the clock, so
    // the two runs agree to well under 1e-6 rather than bit-for-bit.
    assert_eq!(
        got.len(),
        expected.len(),
        "got {got:?}, expected {expected:?}"
    );
    for ((gid, gs), (eid, es)) in got.iter().zip(&expected) {
        assert_eq!(gid, eid, "got {got:?}, expected {expected:?}");
        assert!((gs - es).abs() < 1e-6, "got {got:?}, expected {expected:?}");
    }
}

#[test]
fn run_reports_zero_hits_for_an_unmatched_query() {
    let home = tempfile::tempdir().expect("tempdir");
    save(&home, "unrelated body about caching");
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let result = retrieval::context::run(&mut ctx, request("zzz_never_matches_zzz"), false)
        .expect("context run");
    assert!(result.bundle.memories.is_empty());
}

#[test]
fn track_true_bumps_memory_access_count() {
    let home = tempfile::tempdir().expect("tempdir");
    save(&home, "advisory lock guards the migration runner");
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();

    let before: i64 = conn
        .query_row("SELECT access_count FROM memories LIMIT 1", [], |r| {
            r.get(0)
        })
        .expect("read access_count before");
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        retrieval::context::run(&mut ctx, request("advisory lock"), true).expect("context run");
    }
    let after: i64 = conn
        .query_row("SELECT access_count FROM memories LIMIT 1", [], |r| {
            r.get(0)
        })
        .expect("read access_count after");
    assert!(after > before, "track=true must bump access_count");
}

/// Save a memory through the real binary and return the id it reports.
fn save_id(home: &tempfile::TempDir, body: &str) -> String {
    let assert = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save", body, "--kind", "decision", "--repo", "demo", "--json",
        ])
        .assert()
        .success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(&out).expect("save --json");
    v["id"].as_str().expect("saved id").to_string()
}

/// Insert one real `code_symbols` row through the production writer, so the
/// column defaults (`rank_score`, `access_count`, `last_accessed`) are the
/// ones `index-code` produces.
fn seed_symbol(conn: &rusqlite::Connection, path: &str, symbol: &str) {
    code_row::insert(
        conn,
        &CodeSymbolRow {
            repo: "demo",
            path,
            blob_oid: "oid",
            symbol,
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 10,
            snippet: "fn body() {}",
            simhash: 0,
            parent_id: None,
        },
    )
    .expect("insert code symbol");
}

/// Point `memory_id` at a `<repo>:<path>:<symbol>` destination, the shape
/// `bundle::assemble` resolves into `resolved_code_ids`.
fn seed_symbol_edge(conn: &rusqlite::Connection, memory_id: &str, dst: &str) {
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES('memory',?1,'symbol',?2,'references_symbol','t')",
        rusqlite::params![memory_id, dst],
    )
    .expect("seed references_symbol edge");
}

/// Symbols carrying any access-tracking write at all — either column.
fn bumped_symbols(conn: &rusqlite::Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM code_symbols WHERE access_count <> 0 OR last_accessed IS NOT NULL",
        [],
        |r| r.get(0),
    )
    .expect("count bumped symbols")
}

/// Issue #201: `context`'s code-ref self-reinforcement must also stop at the
/// head of the ranking.
///
/// This bump is NOT covered by `pipeline::search`'s gate — it fires after the
/// bundle is assembled, over refs derived from the page that was returned —
/// so a deep `context` page would keep churning `code_prior`'s activation
/// input and reordering code refs between identical calls.
///
/// Both halves are deltas over one corpus: the offset-0 call must bump
/// (proving the refs really resolve, so the negative half is not vacuous),
/// and the offset-1 call must then add nothing.
#[test]
fn a_deep_context_page_does_not_bump_code_ref_access_counts() {
    let home = tempfile::tempdir().expect("tempdir");
    let first = save_id(&home, "advisory lock guards the migration runner");
    let second = save_id(&home, "advisory lock protects every schema rebuild");
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    seed_symbol(&conn, "alpha.rs", "alpha_run");
    seed_symbol(&conn, "bravo.rs", "bravo_run");
    seed_symbol_edge(&conn, &first, "demo:alpha.rs:alpha_run");
    seed_symbol_edge(&conn, &second, "demo:bravo.rs:bravo_run");
    let cfg = Config::defaults();

    let paged = |conn: &mut rusqlite::Connection, offset: usize| {
        let mut ctx = Ctx::borrowed(&paths, &cfg, conn);
        retrieval::context::run(
            &mut ctx,
            retrieval::context::Request {
                k: Some(1),
                offset,
                ..request("advisory lock")
            },
            true,
        )
        .expect("context run")
    };

    let head = paged(&mut conn, 0);
    assert_eq!(head.bundle.memories.len(), 1, "a one-row head page");
    let after_head = bumped_symbols(&conn);
    assert!(
        after_head > 0,
        "the head page must reinforce its resolved code ref, else the \
         deep-page assertion below would prove nothing"
    );

    let deep = paged(&mut conn, 1);
    assert_eq!(deep.bundle.memories.len(), 1, "a one-row page at offset 1");
    assert_eq!(
        bumped_symbols(&conn),
        after_head,
        "a context page past the head must not reinforce further code refs"
    );
}
