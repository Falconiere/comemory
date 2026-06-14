//! Task 12: `comemory index-code` — part 2.
//!
//! Covers: the blob-OID cursor short-circuit on re-run, cAST chunk-child
//! persistence for oversized symbols, the per-repo format-stamp mismatch
//! forced re-extraction, and the `--extract` JSONL contract.

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;
#[path = "common/git_sample.rs"]
mod git_sample;

use assert_cmd::Command;
use serde::Deserialize;
use tempfile::tempdir;

/// Real oversized function fixture (copied from this repo's
/// `src/config/env.rs::with_env`) — shared with `tests/ast/chunk.rs` so
/// the CLI tests exercise the same chunking corpus end to end.
const OVERSIZED_SRC: &str = include_str!("ast/fixtures/oversized_fn.rs");

/// Build a repo whose single source file holds the oversized fixture fn,
/// returning the working-tree root.
fn build_oversized_repo(root: &std::path::Path) -> std::path::PathBuf {
    let repo = root.join("oversized-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("big.rs", OVERSIZED_SRC)], "init");
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
    let repo = git_sample::build_sample_repo(workspace.path());

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
    let repo = git_sample::build_sample_repo(workspace.path());

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
