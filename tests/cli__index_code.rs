//! Task 12: `comemory index-code` now walks a real git repo and uses each
//! file's blob OID as the indexing cursor.
//!
//! Part 1: the code-graph post-pass — co-change + imports edges, projected
//! PageRank, the mining cursor, and the deterministic re-run no-op.

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use assert_cmd::Command;
use tempfile::tempdir;

/// Build the three-file graph fixture for the post-pass tests: `a.rs`
/// declares `mod b;` (an import the resolver maps a.rs → b.rs), `c.rs`
/// lands alone in the first commit (isolated — no co-change partner, no
/// imports), and a.rs+b.rs are then committed together TWICE with
/// distinct content so the miner counts the pair with weight 2.
fn build_graph_repo(root: &std::path::Path) -> std::path::PathBuf {
    let repo = root.join("graph-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("c.rs", "fn gamma() {}\n")], "c alone");
    git_commit::commit_files(
        &repo,
        &[
            ("a.rs", "mod b;\n\nfn alpha() {}\n"),
            ("b.rs", "fn beta() {}\n"),
        ],
        "couple a+b once",
    );
    git_commit::commit_files(
        &repo,
        &[
            ("a.rs", "mod b;\n\nfn alpha() { let _x = 1; }\n"),
            ("b.rs", "fn beta() { let _y = 2; }\n"),
        ],
        "couple a+b twice",
    );
    repo
}

/// Full `(path, rank_score)` projection for one repo, sorted by path —
/// the determinism probe used by the post-pass test.
fn rank_scores(conn: &rusqlite::Connection, repo: &str) -> Vec<(String, f64)> {
    conn.prepare("SELECT DISTINCT path, rank_score FROM code_symbols WHERE repo = ?1 ORDER BY path")
        .expect("prepare rank_scores query")
        .query_map([repo], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query rank_scores")
        .collect::<Result<_, _>>()
        .expect("collect rank_scores")
}

#[test]
fn index_code_materializes_graph_and_pagerank() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = build_graph_repo(workspace.path());

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "r", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();

    let conn = rusqlite::Connection::open(home.path().join("comemory.db")).expect("open db");

    // Co-change: a.rs+b.rs were committed together twice → one undirected
    // edge in canonical (a < b) order with accumulated weight 2.
    let cochange_weight: i64 = conn
        .query_row(
            "SELECT weight FROM edges WHERE src_kind='file' AND src_id='file:r:a.rs' \
             AND dst_kind='file' AND dst_id='file:r:b.rs' AND rel='co_changed'",
            [],
            |r| r.get(0),
        )
        .expect("co_changed edge a.rs↔b.rs exists");
    assert_eq!(cochange_weight, 2, "two coupling commits → weight 2");

    // Imports: `mod b;` in a.rs resolves to b.rs → directed imports edge.
    let import_edges: i64 = conn
        .query_row(
            "SELECT count(*) FROM edges WHERE src_kind='file' AND src_id='file:r:a.rs' \
             AND dst_kind='file' AND dst_id='file:r:b.rs' AND rel='imports'",
            [],
            |r| r.get(0),
        )
        .expect("count imports edges");
    assert_eq!(import_edges, 1, "exactly one a.rs → b.rs imports edge");

    // PageRank projected onto every symbol row: nothing is left at the
    // 0.0 column default (even isolated c.rs gets the teleport baseline).
    let unranked: i64 = conn
        .query_row(
            "SELECT count(*) FROM code_symbols WHERE repo='r' AND rank_score <= 0.0",
            [],
            |r| r.get(0),
        )
        .expect("count unranked symbols");
    assert_eq!(unranked, 0, "every code_symbols row gets a rank_score > 0");

    // b.rs receives an imports edge plus co-change mass; c.rs is isolated,
    // so b.rs's symbols must outrank c.rs's.
    let score_of = |path: &str| -> f64 {
        conn.query_row(
            "SELECT rank_score FROM code_symbols WHERE repo='r' AND path=?1 LIMIT 1",
            [path],
            |r| r.get(0),
        )
        .expect("rank_score for path")
    };
    assert!(
        score_of("b.rs") > score_of("c.rs"),
        "connected b.rs ({}) must outrank isolated c.rs ({})",
        score_of("b.rs"),
        score_of("c.rs"),
    );

    // Mining cursor advanced to HEAD.
    let cursor: String = conn
        .query_row(
            "SELECT last_mined_commit FROM repo_marker WHERE repo='r'",
            [],
            |r| r.get(0),
        )
        .expect("repo_marker.last_mined_commit set");
    assert_eq!(
        cursor,
        comemory::git_utils::current_head(&repo).expect("HEAD")
    );

    // Re-run with no changes: the cursor makes mining a no-op, weights do
    // not accumulate further, and the recomputed PageRank is byte-identical.
    let before = rank_scores(&conn, "r");
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "r", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();
    let after = rank_scores(&conn, "r");
    assert_eq!(before, after, "re-run must leave rank scores untouched");
    let weight_after: i64 = conn
        .query_row(
            "SELECT weight FROM edges WHERE src_id='file:r:a.rs' \
             AND dst_id='file:r:b.rs' AND rel='co_changed'",
            [],
            |r| r.get(0),
        )
        .expect("co_changed edge after re-run");
    assert_eq!(
        weight_after, 2,
        "cursor no-op: weight must not re-accumulate"
    );
}
