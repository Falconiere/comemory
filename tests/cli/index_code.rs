//! Task 12: `comemory index-code` now walks a real git repo and uses each
//! file's blob OID as the indexing cursor. The second run over an unchanged
//! repo must short-circuit on the `indexed_files` table so `code_symbols`
//! does not grow.

use assert_cmd::Command;
use serde::Deserialize;
use tempfile::tempdir;

use super::git_setup;

/// Build the three-file graph fixture for the post-pass tests: `a.rs`
/// declares `mod b;` (an import the resolver maps a.rs → b.rs), `c.rs`
/// lands alone in the first commit (isolated — no co-change partner, no
/// imports), and a.rs+b.rs are then committed together TWICE with
/// distinct content so the miner counts the pair with weight 2.
fn build_graph_repo(root: &std::path::Path) -> std::path::PathBuf {
    let repo = root.join("graph-repo");
    git_setup::init_repo(&repo);
    git_setup::commit_files(&repo, &[("c.rs", "fn gamma() {}\n")], "c alone");
    git_setup::commit_files(
        &repo,
        &[
            ("a.rs", "mod b;\n\nfn alpha() {}\n"),
            ("b.rs", "fn beta() {}\n"),
        ],
        "couple a+b once",
    );
    git_setup::commit_files(
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

/// Real oversized function fixture (copied from this repo's
/// `src/config/env.rs::with_env`) — shared with `tests/ast/chunk.rs` so
/// the CLI tests exercise the same chunking corpus end to end.
const OVERSIZED_SRC: &str = include_str!("../ast/fixtures/oversized_fn.rs");

/// Build a repo whose single source file holds the oversized fixture fn,
/// returning the working-tree root.
fn build_oversized_repo(root: &std::path::Path) -> std::path::PathBuf {
    let repo = root.join("oversized-repo");
    git_setup::init_repo(&repo);
    git_setup::commit_files(&repo, &[("big.rs", OVERSIZED_SRC)], "init");
    repo
}

/// Mirrors `cli::ingest_code::Row` minus the `embedding` field. Lets the
/// `--extract` test assert that the JSONL emitted by `index-code` is wire
/// compatible with the ingest path's expected shape — the contract the two
/// commands share. Every field is read in `assert!`s below so deny-by-default
/// dead_code never fires.
#[derive(Deserialize)]
struct ExtractRow {
    repo: String,
    path: String,
    blob_oid: String,
    symbol: String,
    kind: String,
    lang: String,
    line_start: u32,
    line_end: u32,
    snippet: String,
    simhash: i64,
}

#[test]
fn index_code_writes_symbols_and_skips_unchanged_on_rerun() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = git_setup::build_sample_repo(workspace.path());

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "sample", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();

    let db = home.path().join("comemory.db");
    let conn = rusqlite::Connection::open(&db).expect("open db");
    let initial: i64 = conn
        .query_row("SELECT count(*) FROM code_symbols", [], |r| r.get(0))
        .expect("count code_symbols");
    assert!(initial >= 2, "expected >= 2 symbols, got: {initial}");

    // Second run: nothing changed, `indexed_files` row blocks the re-walk so
    // `code_symbols` must stay at the same row count.
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "sample", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();

    let after: i64 = conn
        .query_row("SELECT count(*) FROM code_symbols", [], |r| r.get(0))
        .expect("count code_symbols");
    assert_eq!(after, initial);
}

#[test]
fn index_code_persists_chunk_children_for_oversized_symbols() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = build_oversized_repo(workspace.path());

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "big", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();

    let conn = rusqlite::Connection::open(home.path().join("comemory.db")).expect("open db");
    let parent_id: i64 = conn
        .query_row(
            "SELECT id FROM code_symbols WHERE repo='big' AND symbol='with_env' \
             AND parent_id IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("parent row for with_env");
    let children: Vec<(String, i64, i64)> = conn
        .prepare(
            "SELECT symbol, line_start, line_end FROM code_symbols \
             WHERE repo='big' AND parent_id = ?1 ORDER BY line_start",
        )
        .expect("prepare children query")
        .query_map([parent_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query children")
        .collect::<Result<_, _>>()
        .expect("collect children");
    assert!(
        children.len() >= 2,
        "oversized symbol must split into >= 2 chunk children, got {children:?}",
    );
    for (i, (symbol, start, end)) in children.iter().enumerate() {
        assert_eq!(
            symbol,
            &format!("with_env#{}", i + 1),
            "chunk symbols are <name>#<n>"
        );
        assert!(end >= start, "chunk span inverted: {symbol} {start}..{end}");
    }
    // Parent snippet is the headline: signature line + first chunk.
    let parent_snippet: String = conn
        .query_row(
            "SELECT snippet FROM code_symbols WHERE id = ?1",
            [parent_id],
            |r| r.get(0),
        )
        .expect("parent snippet");
    assert!(
        parent_snippet.starts_with("fn with_env"),
        "headline keeps the signature line: {parent_snippet:?}",
    );
    let budget_plus_headline = comemory::ast::chunk::CHUNK_LINE_BUDGET + 1;
    assert!(
        parent_snippet.lines().count() <= budget_plus_headline,
        "headline snippet stays within signature + first chunk",
    );
}

#[test]
fn index_code_format_stamp_mismatch_forces_reextraction() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = build_oversized_repo(workspace.path());

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "big", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();

    let conn = rusqlite::Connection::open(home.path().join("comemory.db")).expect("open db");
    let stamp: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'code_format:big'",
            [],
            |r| r.get(0),
        )
        .expect("per-repo format stamp written");
    assert_eq!(stamp, "2");
    let first_indexed_at: String = conn
        .query_row(
            "SELECT indexed_at FROM indexed_files WHERE repo='big' AND path='big.rs'",
            [],
            |r| r.get(0),
        )
        .expect("indexed_files cursor exists");

    // Simulate a pre-chunking index: downgrade the stamp and strip the
    // chunk children so the rows look like format-1 output.
    conn.execute(
        "UPDATE schema_meta SET value = '1' WHERE key = 'code_format:big'",
        [],
    )
    .expect("downgrade stamp");
    conn.execute(
        "DELETE FROM code_symbols WHERE repo='big' AND parent_id IS NOT NULL",
        [],
    )
    .expect("strip chunk children");
    drop(conn);

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "big", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();

    let conn = rusqlite::Connection::open(home.path().join("comemory.db")).expect("open db");
    // The stale stamp must have dropped the indexed_files cursor, forcing a
    // fresh extraction: the chunk children are back and the cursor was
    // rewritten with a new indexed_at.
    let children: i64 = conn
        .query_row(
            "SELECT count(*) FROM code_symbols WHERE repo='big' AND parent_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .expect("count children");
    assert!(
        children >= 2,
        "format mismatch must force re-extraction of chunk children, got {children}",
    );
    let second_indexed_at: String = conn
        .query_row(
            "SELECT indexed_at FROM indexed_files WHERE repo='big' AND path='big.rs'",
            [],
            |r| r.get(0),
        )
        .expect("indexed_files cursor rewritten");
    assert_ne!(
        second_indexed_at, first_indexed_at,
        "cursor must be rewritten by the forced re-walk",
    );
    let stamp: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'code_format:big'",
            [],
            |r| r.get(0),
        )
        .expect("stamp restored");
    assert_eq!(stamp, "2", "stamp upgraded back after the walk");
}

#[test]
fn index_code_extract_emits_ingest_compatible_jsonl() {
    // `index-code --extract` is the upstream half of the
    // `index-code | embed | ingest-code` pipeline. The two commands share a
    // JSONL contract: the `--extract` writer emits every field
    // `ingest-code::Row` expects except `embedding` (which the embedder
    // splices in). This test pins that contract — if a new column lands in
    // `code_symbols` and only one side is updated, this round-trip parse
    // fails loudly.
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = git_setup::build_sample_repo(workspace.path());

    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "sample", "--extract", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).expect("utf8 stdout");
    let first_line = stdout
        .lines()
        .find(|l| !l.trim().is_empty())
        .expect("at least one JSONL row on stdout");
    let row: ExtractRow = serde_json::from_str(first_line).expect("parse JSONL row");

    // Every field of the shared contract is read here so the struct can stay
    // dead_code-free without #[allow] attrs.
    assert_eq!(row.repo, "sample");
    assert!(!row.path.is_empty(), "path: {:?}", row.path);
    assert_eq!(
        row.blob_oid.len(),
        40,
        "blob_oid is a 40-hex SHA1: {:?}",
        row.blob_oid
    );
    assert!(!row.symbol.is_empty(), "symbol: {:?}", row.symbol);
    assert!(!row.kind.is_empty(), "kind: {:?}", row.kind);
    assert_eq!(row.lang, "rust", "lang: {:?}", row.lang);
    assert!(
        row.line_end >= row.line_start,
        "line_end ({}) must be >= line_start ({})",
        row.line_end,
        row.line_start,
    );
    assert!(!row.snippet.is_empty(), "snippet: {:?}", row.snippet);
    // `simhash` is i64 — just reading it is enough to confirm presence;
    // its concrete value depends on the snippet tokens.
    let _ = row.simhash;

    // DB-write path was NOT taken under --extract. The connection is still
    // opened (we use the same Paths layout to look up `comemory.db`), but
    // the transaction never writes to `code_symbols`/`code_fts`/`code_vec`
    // or `indexed_files`, so a count of 0 is the contract we pin here.
    let db = home.path().join("comemory.db");
    if db.exists() {
        let conn = rusqlite::Connection::open(&db).expect("open db");
        let symbols: i64 = conn
            .query_row("SELECT count(*) FROM code_symbols", [], |r| r.get(0))
            .expect("count code_symbols");
        assert_eq!(
            symbols, 0,
            "--extract must not insert code_symbols rows, found {symbols}",
        );
    }
}
