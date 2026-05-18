use qwick_memory::config::paths::Paths;
use qwick_memory::index::{CodeIndex, Embedder};

use super::common;

/// End-to-end: write a two-function Rust file, run `index_repo`, and confirm
/// at least two symbols were upserted. Also doubles as a smoke test for the
/// fastembed jina-code cold-download path the first time it runs.
#[tokio::test]
async fn index_repo_finds_rust_symbols() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    let repo = sb.root.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();

    let idx = CodeIndex::open(paths.vectors_dir(), 768).await.unwrap();
    let mut emb = Embedder::jina_code().unwrap();
    let n = idx.index_repo(&repo, "test", &mut emb).await.unwrap();
    assert!(n >= 2, "expected at least 2 symbols, got {n}");
}

/// `index_repo` on a repo with no supported source files must return Ok(0)
/// rather than erroring out or creating an empty table.
#[tokio::test]
async fn index_repo_no_source_files_returns_zero() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    let repo = sb.root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("README.md"), "# nothing here\n").unwrap();

    let idx = CodeIndex::open(paths.vectors_dir(), 768).await.unwrap();
    let mut emb = Embedder::jina_code().unwrap();
    let n = idx.index_repo(&repo, "test", &mut emb).await.unwrap();
    assert_eq!(n, 0);
}

/// Re-indexing the same repo twice must not duplicate rows: `merge_insert`
/// on `qualified` should overwrite the existing row in-place. We don't have
/// a query API on `CodeIndex` yet, so we assert the public count matches the
/// initial pass — the absence of an error from the second `merge_insert` is
/// the schema-side contract; row-level dedup is covered once Task 14's
/// reader lands.
#[tokio::test]
async fn index_repo_is_idempotent_on_qualified_key() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    let repo = sb.root.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();

    let idx = CodeIndex::open(paths.vectors_dir(), 768).await.unwrap();
    let mut emb = Embedder::jina_code().unwrap();
    let first = idx.index_repo(&repo, "test", &mut emb).await.unwrap();
    let second = idx.index_repo(&repo, "test", &mut emb).await.unwrap();
    assert_eq!(
        first, second,
        "second pass should write the same row count, got {first} then {second}"
    );
}
