//! Task 12: `comemory index-code` now walks a real git repo and uses each
//! file's blob OID as the indexing cursor. The second run over an unchanged
//! repo must short-circuit on the `indexed_files` table so `code_symbols`
//! does not grow.

use assert_cmd::Command;
use serde::Deserialize;
use tempfile::tempdir;

#[path = "../common/git_setup.rs"]
mod git_setup;

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

    // DB-write path was NOT taken under --extract: comemory.db should not
    // even exist (the connection::open call was bypassed).
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
