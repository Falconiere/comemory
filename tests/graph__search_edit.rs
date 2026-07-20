//! Search→edit lookback (`comemory::graph::search_edit`) exercised through
//! the public `materialize` path. `search_edit` is `pub(crate)`, so these
//! tests assert provenance upgrades the same way `index-code` does: seed a
//! `retrieval_log` hit, then materialize over real git touches.

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use comemory::graph::materialize::materialize;
use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::connection;
use comemory::store::memory_row;
use rusqlite::Connection;
use tempfile::TempDir;
use time::OffsetDateTime;

const REPO: &str = "r";

fn build_repo(root: &Path) -> PathBuf {
    let repo = root.join("search-edit-repo");
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

fn seed_search_hit(conn: &Connection, memory_id: &str) {
    let at = memory_row::iso_format(OffsetDateTime::now_utc()).expect("iso now");
    let returned = serde_json::to_string(&vec![memory_id]).expect("json ids");
    conn.execute(
        "INSERT INTO retrieval_log(query_id, query, returned_ids, at, duration_ms, repo, source) \
         VALUES ('q-20260720-aabbccdd', 'guide docs', ?1, ?2, 1, ?3, 'search')",
        rusqlite::params![returned, at, REPO],
    )
    .expect("insert retrieval_log");
}

fn used_event(conn: &Connection, id: &str) -> Option<(String, String)> {
    conn.query_row(
        "SELECT provenance, query_id FROM feedback_events \
          WHERE memory_id=?1 AND verdict='used'",
        rusqlite::params![id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .ok()
}

/// A memory returned by a recent `search` page that also crosses the Beta
/// threshold gets `auto_search_edit` / `auto-search-edit`. A sibling memory
/// on the same touched file without a retrieval_log hit stays
/// `auto_coactivation`. Golden harvest stays empty for both auto rows.
#[test]
fn search_hit_upgrades_provenance_while_miss_stays_coactivation() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let mut conn = open_db_with_symbols(&home);

    // Both reference the twice-touched file → both cross Beta in one pass.
    seed_memory_referencing(&conn, "aaaaaaa1", "docs/guide.md");
    seed_memory_referencing(&conn, "aaaaaaa2", "docs/guide.md");
    seed_search_hit(&conn, "aaaaaaa1");

    materialize(&mut conn, &repo_root, REPO, &BTreeMap::new(), 7).expect("materialize");

    let (prov1, qid1) = used_event(&conn, "aaaaaaa1").expect("search-edit used event");
    assert_eq!(prov1, "auto_search_edit");
    assert_eq!(qid1, "auto-search-edit");

    let (prov2, qid2) = used_event(&conn, "aaaaaaa2").expect("coactivation used event");
    assert_eq!(prov2, "auto_coactivation");
    assert_eq!(qid2, "auto-coactivation");

    let pairs = comemory::eval::golden::harvest(&conn).expect("golden harvest");
    assert!(
        pairs.is_empty(),
        "auto provenance rows must not mint golden pairs, got {pairs:?}"
    );
}
