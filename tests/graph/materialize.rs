//! Integration tests for `comemory::graph::materialize` against a REAL
//! git repo (built with the git CLI) and a real `comemory.db` connection.
//! The fixture mirrors the CLI-level test: `a.rs` imports `b.rs` (`mod b;`
//! raw module "b"), a.rs+b.rs are committed together twice, `c.rs` is
//! committed alone (isolated node).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use comemory::git_utils::current_head;
use comemory::graph::materialize::materialize;
use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::connection;
use tempfile::TempDir;

use crate::git_setup;

/// Build the three-file coupling fixture and return the repo root.
fn build_repo(root: &Path) -> PathBuf {
    let repo = root.join("graph-repo");
    git_setup::init_repo(&repo);
    git_setup::commit_files(&repo, &[("c.rs", "fn gamma() {}\n")], "c alone");
    git_setup::commit_files(
        &repo,
        &[("a.rs", "mod b;\n"), ("b.rs", "fn beta() {}\n")],
        "c1",
    );
    git_setup::commit_files(
        &repo,
        &[
            ("a.rs", "mod b;\nfn alpha() {}\n"),
            ("b.rs", "fn beta() { let _x = 1; }\n"),
        ],
        "c2",
    );
    repo
}

/// Open a fresh `comemory.db` and insert one `code_symbols` row per file
/// via the production writer, so `materialize` sees the indexed paths.
fn seed_symbols(dir: &TempDir, repo: &str, paths: &[&str]) -> rusqlite::Connection {
    let conn = connection::open(dir.path().join("comemory.db")).expect("open db");
    for path in paths {
        code_row::insert(
            &conn,
            &CodeSymbolRow {
                repo,
                path,
                blob_oid: "0000000000000000000000000000000000000000",
                symbol: "sym",
                kind: "function",
                lang: "rust",
                line_start: 1,
                line_end: 1,
                snippet: "fn sym() {}",
                simhash: 0,
                parent_id: None,
            },
        )
        .expect("insert code_symbols row");
    }
    conn
}

/// `imports_by_file` map carrying a.rs's raw `mod b;` module.
fn imports_a_to_b() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([("a.rs".to_string(), vec!["b".to_string()])])
}

#[test]
fn materialize_writes_edges_ranks_and_cursor() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let mut conn = seed_symbols(&home, "r", &["a.rs", "b.rs", "c.rs"]);

    materialize(&mut conn, &repo_root, "r", &imports_a_to_b()).expect("materialize");

    let weight: i64 = conn
        .query_row(
            "SELECT weight FROM edges WHERE src_id='file:r:a.rs' \
             AND dst_id='file:r:b.rs' AND rel='co_changed'",
            [],
            |r| r.get(0),
        )
        .expect("co_changed edge");
    assert_eq!(weight, 2, "two coupling commits → weight 2");

    let imports: i64 = conn
        .query_row(
            "SELECT count(*) FROM edges WHERE src_id='file:r:a.rs' \
             AND dst_id='file:r:b.rs' AND rel='imports'",
            [],
            |r| r.get(0),
        )
        .expect("imports edge count");
    assert_eq!(imports, 1);

    let unranked: i64 = conn
        .query_row(
            "SELECT count(*) FROM code_symbols WHERE repo='r' AND rank_score <= 0.0",
            [],
            |r| r.get(0),
        )
        .expect("unranked count");
    assert_eq!(unranked, 0, "every symbol row receives a positive rank");

    let cursor: String = conn
        .query_row(
            "SELECT last_mined_commit FROM repo_marker WHERE repo='r'",
            [],
            |r| r.get(0),
        )
        .expect("cursor row");
    assert_eq!(cursor, current_head(&repo_root).expect("HEAD"));
}

#[test]
fn materialize_accumulates_cochange_incrementally() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let mut conn = seed_symbols(&home, "r", &["a.rs", "b.rs", "c.rs"]);
    let no_imports = BTreeMap::new();

    materialize(&mut conn, &repo_root, "r", &no_imports).expect("first run");
    // No new commits: the cursor short-circuits mining, weight stays 2.
    materialize(&mut conn, &repo_root, "r", &no_imports).expect("no-op run");
    let weight: i64 = conn
        .query_row(
            "SELECT weight FROM edges WHERE src_id='file:r:a.rs' \
             AND dst_id='file:r:b.rs' AND rel='co_changed'",
            [],
            |r| r.get(0),
        )
        .expect("co_changed edge");
    assert_eq!(weight, 2, "cursor no-op must not re-accumulate");

    // One more coupling commit: only the NEW commit is walked → weight 3.
    git_setup::commit_files(
        &repo_root,
        &[
            ("a.rs", "mod b;\nfn alpha2() {}\n"),
            ("b.rs", "fn beta2() {}\n"),
        ],
        "c3",
    );
    materialize(&mut conn, &repo_root, "r", &no_imports).expect("incremental run");
    let weight: i64 = conn
        .query_row(
            "SELECT weight FROM edges WHERE src_id='file:r:a.rs' \
             AND dst_id='file:r:b.rs' AND rel='co_changed'",
            [],
            |r| r.get(0),
        )
        .expect("co_changed edge");
    assert_eq!(weight, 3, "new coupling commit adds exactly 1");
    let cursor: String = conn
        .query_row(
            "SELECT last_mined_commit FROM repo_marker WHERE repo='r'",
            [],
            |r| r.get(0),
        )
        .expect("cursor row");
    assert_eq!(cursor, current_head(&repo_root).expect("HEAD after c3"));
}

/// A stored cursor that no longer resolves (history rewrite + gc, or a
/// corrupted marker row) makes the miner re-count bounded history from
/// scratch; materialize must RESET the repo's accumulated `co_changed`
/// weights before applying the re-mined pairs — accumulating on top would
/// double-count every pair that survived the rewrite.
#[test]
fn materialize_resets_cochange_weights_when_cursor_is_lost() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let mut conn = seed_symbols(&home, "r", &["a.rs", "b.rs", "c.rs"]);
    let no_imports = BTreeMap::new();

    materialize(&mut conn, &repo_root, "r", &no_imports).expect("first run");
    // Corrupt the cursor to a well-formed oid that names no object —
    // the same observable state a rebase/amend + gc leaves behind.
    conn.execute(
        "UPDATE repo_marker SET last_mined_commit = \
         '0123456789abcdef0123456789abcdef01234567' WHERE repo = 'r'",
        [],
    )
    .expect("corrupt cursor");

    materialize(&mut conn, &repo_root, "r", &no_imports).expect("lost-cursor run");
    let weight: i64 = conn
        .query_row(
            "SELECT weight FROM edges WHERE src_id='file:r:a.rs' \
             AND dst_id='file:r:b.rs' AND rel='co_changed'",
            [],
            |r| r.get(0),
        )
        .expect("co_changed edge");
    assert_eq!(
        weight, 2,
        "lost cursor must reset weights before re-applying (not 4 = double-count)"
    );
    // The cursor heals to the current HEAD so the next run is incremental.
    let cursor: String = conn
        .query_row(
            "SELECT last_mined_commit FROM repo_marker WHERE repo='r'",
            [],
            |r| r.get(0),
        )
        .expect("cursor row");
    assert_eq!(cursor, current_head(&repo_root).expect("HEAD"));
}

#[test]
fn materialize_refreshes_imports_as_state_not_accumulation() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let mut conn = seed_symbols(&home, "r", &["a.rs", "b.rs", "c.rs"]);

    materialize(&mut conn, &repo_root, "r", &imports_a_to_b()).expect("first run");
    // Same imports again: still exactly one edge (INSERT OR IGNORE).
    materialize(&mut conn, &repo_root, "r", &imports_a_to_b()).expect("repeat run");
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM edges WHERE src_id='file:r:a.rs' AND rel='imports'",
            [],
            |r| r.get(0),
        )
        .expect("imports count");
    assert_eq!(count, 1);

    // a.rs re-indexed with its import dropped: the old edge must vanish.
    let dropped = BTreeMap::from([("a.rs".to_string(), Vec::<String>::new())]);
    materialize(&mut conn, &repo_root, "r", &dropped).expect("dropped-import run");
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM edges WHERE src_id='file:r:a.rs' AND rel='imports'",
            [],
            |r| r.get(0),
        )
        .expect("imports count after drop");
    assert_eq!(
        count, 0,
        "imports edges are state — refreshed, not accumulated"
    );
}

#[test]
fn materialize_without_indexed_symbols_is_a_noop_and_keeps_no_cursor() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let mut conn = connection::open(home.path().join("comemory.db")).expect("open db");

    materialize(&mut conn, &repo_root, "r", &BTreeMap::new()).expect("empty-repo run");

    let edges: i64 = conn
        .query_row("SELECT count(*) FROM edges", [], |r| r.get(0))
        .expect("edges count");
    assert_eq!(edges, 0);
    // No cursor row: advancing it before any file is indexed would skip
    // the existing history once symbols do land.
    let cursor: Option<String> = conn
        .query_row(
            "SELECT last_mined_commit FROM repo_marker WHERE repo='r'",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(cursor, None, "cursor must not advance past unmined history");
}

#[test]
fn materialize_on_commitless_repo_errors_and_rolls_back() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = workspace.path().join("empty-repo");
    git_setup::init_repo(&repo_root);
    let mut conn = seed_symbols(&home, "r", &["a.rs"]);

    let err = materialize(&mut conn, &repo_root, "r", &BTreeMap::new());
    assert!(
        err.is_err(),
        "unborn HEAD must surface as Err for the caller's warn path"
    );
    let edges: i64 = conn
        .query_row("SELECT count(*) FROM edges", [], |r| r.get(0))
        .expect("edges count");
    assert_eq!(
        edges, 0,
        "failed materialization must leave no partial graph rows"
    );
}
