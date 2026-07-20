//! End-to-end tests for the commit co-activation reward
//! (`comemory::graph::coactivate`) driven through the public
//! `materialize::materialize` entry against a REAL git repo and a real
//! `comemory.db`. No mocks: commits touch real files, memories carry real
//! `references_file` edges, and every channel is asserted by querying the
//! committed database.
//!
//! `coactivate::harvest` is `pub(crate)`, so it is exercised here through
//! `materialize` (the production call site) — the same path `index-code`
//! takes. The fixture references the BARE `<repo>:<path>` dst_id form the
//! cross-link writer uses, which is what the reverse query matches.

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use comemory::graph::materialize::materialize;
use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::connection;
use rusqlite::Connection;
use tempfile::TempDir;

const REPO: &str = "r";

/// Build a repo where `docs/guide.md` is touched TWICE (crosses the Beta
/// threshold in one pass), `notes.md` ONCE (no crossing), and `untouched.md`
/// never. A `.rs` file rides along so `code_symbols` has a row to index.
fn build_repo(root: &Path) -> PathBuf {
    let repo = root.join("coact-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[("a.rs", "fn a() {}\n"), ("docs/guide.md", "v1\n")],
        "c1",
    );
    git_commit::commit_files(
        &repo,
        &[("docs/guide.md", "v2\n"), ("notes.md", "n1\n")],
        "c2",
    );
    repo
}

/// Open a fresh `comemory.db` and seed one `code_symbols` row so
/// `materialize` has indexed paths (otherwise it no-ops). Only `a.rs` is a
/// code file; the markdown files are referenced by memories, not indexed.
fn open_db_with_symbols(home: &TempDir) -> Connection {
    let conn = connection::open(home.path().join("comemory.db")).expect("open db");
    code_row::insert(
        &conn,
        &CodeSymbolRow {
            repo: REPO,
            path: "a.rs",
            blob_oid: "0000000000000000000000000000000000000000",
            symbol: "a",
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 1,
            snippet: "fn a() {}",
            simhash: 0,
            parent_id: None,
        },
    )
    .expect("insert code_symbols row");
    conn
}

/// Insert a live `memories` row (only the NOT NULL columns matter here) and a
/// `references_file` edge to `<repo>:<path>` in the BARE cross-link form.
fn seed_memory_referencing(conn: &Connection, id: &str, path: &str) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, content_hash, body, created_at, updated_at, md_path) \
         VALUES (?1, ?1, 'note', 'h', 'b', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', ?1)",
        rusqlite::params![id],
    )
    .expect("insert memory");
    conn.execute(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at) \
         VALUES ('memory', ?1, 'file', ?2, 'references_file', '2026-01-01T00:00:00Z')",
        rusqlite::params![id, format!("{REPO}:{path}")],
    )
    .expect("insert references_file edge");
}

fn co_activated_weight(conn: &Connection, id: &str, path: &str) -> Option<i64> {
    conn.query_row(
        "SELECT weight FROM edges WHERE src_kind='memory' AND src_id=?1 \
          AND dst_id=?2 AND rel='co_activated'",
        rusqlite::params![id, format!("file:{REPO}:{path}")],
        |r| r.get(0),
    )
    .ok()
}

fn access_count(conn: &Connection, id: &str) -> i64 {
    conn.query_row(
        "SELECT access_count FROM memories WHERE id=?1",
        rusqlite::params![id],
        |r| r.get(0),
    )
    .expect("access_count")
}

fn last_accessed(conn: &Connection, id: &str) -> Option<String> {
    conn.query_row(
        "SELECT last_accessed FROM memories WHERE id=?1",
        rusqlite::params![id],
        |r| r.get(0),
    )
    .expect("last_accessed")
}

fn coactivation_used_events(conn: &Connection, id: &str) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM feedback_events \
          WHERE memory_id=?1 AND verdict='used' AND provenance='auto_coactivation'",
        rusqlite::params![id],
        |r| r.get(0),
    )
    .expect("count events")
}

/// All three channels fire for a referenced+touched file: the co_activated
/// edge weight equals the touch count, the memory's activation bumps, and —
/// when the weight crosses >= 2 — exactly one provenance-tagged `used` is
/// minted. A memory referencing an UNTOUCHED file gets nothing.
#[test]
fn harvest_rewards_referenced_touched_files_only() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let conn = open_db_with_symbols(&home);

    seed_memory_referencing(&conn, "aaaaaaa1", "docs/guide.md"); // touched x2
    seed_memory_referencing(&conn, "aaaaaaa2", "notes.md"); // touched x1
    seed_memory_referencing(&conn, "aaaaaaa3", "untouched.md"); // touched x0

    let mut conn = conn;
    materialize(&mut conn, &repo_root, REPO, &BTreeMap::new(), 7).expect("materialize");

    // (a) edge weight equals commit-touch count.
    assert_eq!(
        co_activated_weight(&conn, "aaaaaaa1", "docs/guide.md"),
        Some(2),
        "guide.md touched in c1+c2"
    );
    assert_eq!(
        co_activated_weight(&conn, "aaaaaaa2", "notes.md"),
        Some(1),
        "notes.md touched in c2 only"
    );

    // (b) activation bumped + last_accessed set for reinforced memories.
    assert_eq!(
        access_count(&conn, "aaaaaaa1"),
        1,
        "guide memory bumped once"
    );
    assert_eq!(
        access_count(&conn, "aaaaaaa2"),
        1,
        "notes memory bumped once"
    );
    assert!(
        last_accessed(&conn, "aaaaaaa1").is_some(),
        "reinforced memory gets last_accessed"
    );

    // (c) Beta `used` minted exactly once when weight crosses >= 2.
    assert_eq!(
        coactivation_used_events(&conn, "aaaaaaa1"),
        1,
        "weight 0->2 crosses the Beta threshold once"
    );
    assert_eq!(
        coactivation_used_events(&conn, "aaaaaaa2"),
        0,
        "weight 0->1 does not reach the Beta threshold"
    );

    // (d) the untouched-file memory is untouched on every channel.
    assert_eq!(co_activated_weight(&conn, "aaaaaaa3", "untouched.md"), None);
    assert_eq!(access_count(&conn, "aaaaaaa3"), 0, "no reward, no bump");
    assert_eq!(coactivation_used_events(&conn, "aaaaaaa3"), 0);
}

/// Idempotency: re-running materialize over the SAME history must not
/// double-count. The mining cursor short-circuits the second pass, so the
/// touch map is empty, the edge weight holds, the activation does not bump
/// again, and no second `used` is minted.
#[test]
fn rerun_does_not_double_count() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let mut conn = open_db_with_symbols(&home);
    seed_memory_referencing(&conn, "aaaaaaa1", "docs/guide.md");

    materialize(&mut conn, &repo_root, REPO, &BTreeMap::new(), 7).expect("first run");
    let weight_after_first = co_activated_weight(&conn, "aaaaaaa1", "docs/guide.md");
    let access_after_first = access_count(&conn, "aaaaaaa1");

    materialize(&mut conn, &repo_root, REPO, &BTreeMap::new(), 7).expect("second run (no-op mine)");

    assert_eq!(
        co_activated_weight(&conn, "aaaaaaa1", "docs/guide.md"),
        weight_after_first,
        "cursor blocks re-harvest: weight must not grow"
    );
    assert_eq!(weight_after_first, Some(2));
    assert_eq!(
        access_count(&conn, "aaaaaaa1"),
        access_after_first,
        "no new touches → no second activation bump"
    );
    assert_eq!(
        coactivation_used_events(&conn, "aaaaaaa1"),
        1,
        "the Beta `used` is minted exactly once across re-runs"
    );
}

/// An incremental third commit touching `docs/guide.md` again adds exactly 1
/// to the edge weight (cursor walks only the NEW commit) but mints NO second
/// `used` — the Beta crossing already fired when the weight first reached 2.
#[test]
fn incremental_commit_accumulates_weight_without_reminting_used() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let mut conn = open_db_with_symbols(&home);
    seed_memory_referencing(&conn, "aaaaaaa1", "docs/guide.md");

    materialize(&mut conn, &repo_root, REPO, &BTreeMap::new(), 7).expect("first run");
    git_commit::commit_files(&repo_root, &[("docs/guide.md", "v3\n")], "c3");
    materialize(&mut conn, &repo_root, REPO, &BTreeMap::new(), 7).expect("incremental run");

    assert_eq!(
        co_activated_weight(&conn, "aaaaaaa1", "docs/guide.md"),
        Some(3),
        "the new commit adds exactly 1 to the weight"
    );
    assert_eq!(
        coactivation_used_events(&conn, "aaaaaaa1"),
        1,
        "no re-mint: the crossing fired once at weight 2"
    );
}

/// Golden-exclusion guard: the sentinel `query_id` on co-activation feedback
/// has no `retrieval_log` row, so `eval::golden::harvest` (an INNER JOIN on
/// query_id) mints NO golden pair from an auto-reinforced memory — closing
/// the confirmation loop where the system would teach itself its own guesses.
#[test]
fn coactivation_feedback_is_excluded_from_golden_harvest() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let mut conn = open_db_with_symbols(&home);
    seed_memory_referencing(&conn, "aaaaaaa1", "docs/guide.md");

    materialize(&mut conn, &repo_root, REPO, &BTreeMap::new(), 7).expect("materialize");

    // Precondition: a co-activation `used` event exists (sentinel query_id).
    assert_eq!(
        coactivation_used_events(&conn, "aaaaaaa1"),
        1,
        "fixture must produce the auto-coactivation event the guard excludes"
    );
    let pairs = comemory::eval::golden::harvest(&conn).expect("golden harvest");
    assert!(
        pairs.is_empty(),
        "sentinel-query_id feedback must not mint a golden pair, got {pairs:?}"
    );
}
