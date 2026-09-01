#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `api::suggest::run` against a real store (console-api spec §3): real
//! `query_expansions` rows and real `retrieval_log` rows written by real
//! tracked searches through `api::find` / `api::search_code` — never a
//! hand-forged log row, because the exclusion of `search-code` queries is
//! precisely what one of these tests is about.

use comemory::api::{Ctx, find, save, search_code, suggest};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::store::connection;
use tempfile::TempDir;

fn fresh_paths(dir: &TempDir) -> Paths {
    let paths = Paths::new(dir.path().to_path_buf());
    paths.ensure_dirs().unwrap();
    paths
}

fn seed_memory(paths: &Paths, conn: &mut rusqlite::Connection, body: &str) {
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(paths, &cfg, conn);
    save::run(
        &mut ctx,
        save::Request {
            body: body.to_string(),
            title: None,
            kind: Kind::Note,
            repo: "app".into(),
            tags: Vec::new(),
            author: String::new(),
            quality: 3,
            supersedes: Vec::new(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        },
        false,
        None,
    )
    .unwrap();
}

/// One mined expansion row, in the shape `eval::mine` writes.
fn seed_expansion(conn: &rusqlite::Connection, term: &str, expansion: &str, support: i64) {
    conn.execute(
        "INSERT INTO query_expansions(term, expansion, support, last_mined) \
         VALUES (?1, ?2, ?3, '2026-08-01T00:00:00Z')",
        rusqlite::params![term, expansion, support],
    )
    .unwrap();
}

/// Run a real tracked memory-domain search, which writes one
/// `source='find'` row into `retrieval_log`.
fn tracked_find(paths: &Paths, conn: &mut rusqlite::Connection, query: &str) {
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(paths, &cfg, conn);
    find::run(
        &mut ctx,
        find::Request {
            query: query.to_string(),
            k: None,
            offset: 0,
            domain: Some("memory".into()),
            repo: None,
            kind: None,
            lang: None,
            path: Vec::new(),
            vector: None,
            since: None,
            until: None,
            as_of: None,
        },
        true,
    )
    .unwrap();
}

/// Run a real tracked code search, which writes one `source='search-code'`
/// row — the source `suggest` must not offer back.
fn tracked_code_search(paths: &Paths, conn: &mut rusqlite::Connection, query: &str) {
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(paths, &cfg, conn);
    search_code::run(
        &mut ctx,
        search_code::Request {
            query: query.to_string(),
            k: None,
            offset: 0,
            repo: None,
            lang: None,
            vector: None,
        },
        true,
    )
    .unwrap();
}

fn suggest_for(
    paths: &Paths,
    conn: &mut rusqlite::Connection,
    q: &str,
    limit: Option<usize>,
) -> suggest::Response {
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(paths, &cfg, conn);
    suggest::run(
        &mut ctx,
        suggest::Request {
            q: q.to_string(),
            limit,
        },
    )
    .unwrap()
}

#[test]
fn an_empty_query_is_a_bad_request() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let err = suggest::run(
        &mut ctx,
        suggest::Request {
            q: "   ".into(),
            limit: None,
        },
    )
    .expect_err("a blank prefix asks for the whole query log");
    assert!(
        matches!(err, comemory::errors::Error::BadRequest(_)),
        "got {err:?}"
    );
}

#[test]
fn expansions_match_a_query_token_and_come_back_strongest_first() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    seed_expansion(&conn, "auth", "oauth", 2);
    seed_expansion(&conn, "auth", "session", 7);
    seed_expansion(&conn, "billing", "invoice", 9);

    let resp = suggest_for(&paths, &mut conn, "auth flow", None);

    let pairs: Vec<(&str, &str, u64)> = resp
        .expansions
        .iter()
        .map(|e| (e.term.as_str(), e.expansion.as_str(), e.support))
        .collect();
    assert_eq!(
        pairs,
        vec![("auth", "session", 7), ("auth", "oauth", 2)],
        "only the typed query's tokens, strongest support first"
    );
}

#[test]
fn a_query_with_no_mined_terms_returns_no_expansions() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    seed_expansion(&conn, "auth", "oauth", 3);

    let resp = suggest_for(&paths, &mut conn, "deployment", None);

    assert!(
        resp.expansions.is_empty(),
        "an empty list is the honest answer for an unmined query"
    );
}

#[test]
fn recent_offers_prefix_matches_newest_first_and_never_a_code_query() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    seed_memory(&paths, &mut conn, "frontmatter is the contract");

    tracked_find(&paths, &mut conn, "frontmatter contract");
    tracked_find(&paths, &mut conn, "frontmatter rules");
    tracked_code_search(&paths, &mut conn, "frontmatter parser");
    tracked_find(&paths, &mut conn, "unrelated question");

    let logged: i64 = conn
        .query_row("SELECT COUNT(*) FROM retrieval_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(logged, 4, "four real tracked runs were logged");

    let resp = suggest_for(&paths, &mut conn, "frontmatter", None);

    let queries: Vec<&str> = resp.recent.iter().map(|r| r.query.as_str()).collect();
    assert_eq!(
        queries,
        vec!["frontmatter rules", "frontmatter contract"],
        "prefix matches only, newest first, and the search-code run excluded"
    );
    for row in &resp.recent {
        assert!(
            row.query_id.starts_with("q-"),
            "each suggestion carries the retrieval_log id feedback targets: {}",
            row.query_id
        );
        assert!(!row.at.is_empty());
    }
}

#[test]
fn recent_matching_is_case_insensitive_deduplicated_and_limited() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    seed_memory(&paths, &mut conn, "retrieval ladder tiers");

    tracked_find(&paths, &mut conn, "Ladder tiers");
    tracked_find(&paths, &mut conn, "ladder tiers");
    tracked_find(&paths, &mut conn, "ladder fallback");

    let all = suggest_for(&paths, &mut conn, "LADDER", None);
    let queries: Vec<&str> = all.recent.iter().map(|r| r.query.as_str()).collect();
    assert_eq!(
        queries,
        vec!["ladder fallback", "ladder tiers"],
        "`Ladder tiers` and `ladder tiers` are one suggestion, newest kept"
    );

    let capped = suggest_for(&paths, &mut conn, "ladder", Some(1));
    assert_eq!(capped.recent.len(), 1);
    assert_eq!(capped.recent[0].query, "ladder fallback");
}

#[test]
fn a_prefix_holding_a_like_wildcard_matches_literally() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    seed_memory(&paths, &mut conn, "wildcards need escaping");

    tracked_find(&paths, &mut conn, "100% coverage");
    tracked_find(&paths, &mut conn, "1000 coverage");

    let resp = suggest_for(&paths, &mut conn, "100%", None);

    let queries: Vec<&str> = resp.recent.iter().map(|r| r.query.as_str()).collect();
    assert_eq!(
        queries,
        vec!["100% coverage"],
        "an unescaped `%` would have matched `1000 coverage` too"
    );
}
