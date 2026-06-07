use comemory::config::paths::Paths;
use comemory::index::{CodeIndex, Embedder, MemoryIndex};
use comemory::memory::{Kind, MemoryStore};
use comemory::retrieval::fuse::{search_memory_fused_with_fts, FuseOptions};
use comemory::retrieval::hybrid::search_code;

use super::common;

#[tokio::test]
async fn search_returns_results_from_both_layers() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save("postgres migration race fix", Kind::Bug, "r", &[], "a", 3)
        .unwrap();
    let mut text_emb = Embedder::nomic_text().unwrap();
    let v = text_emb.embed_one(&rec.body).unwrap();
    let midx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    midx.upsert(&rec, &v).await.unwrap();

    let repo = sb.root.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/db.rs"), "fn run_migration() {}\n").unwrap();
    let cidx = CodeIndex::open(paths.vectors_dir(), 768).await.unwrap();
    let mut code_emb = Embedder::jina_code().unwrap();
    let written = cidx.index_repo(&repo, "r", &mut code_emb).await.unwrap();
    assert!(
        written > 0,
        "expected at least one code chunk indexed for the dual test"
    );

    let q_text = text_emb.embed_one("postgres migration race").unwrap();
    let q_code = code_emb.embed_one("run_migration").unwrap();
    // Dense-only memory retrieval via the unified fused entry; `fts = None`
    // short-circuits the BM25 path, equivalent to the now-removed
    // `hybrid::search_memory`.
    let mhits = search_memory_fused_with_fts(
        &midx,
        None,
        &paths,
        &q_text,
        "postgres migration race",
        FuseOptions {
            limit: 5,
            dense_threshold: 0.0,
            rrf_k: 60.0,
        },
    )
    .await
    .unwrap();
    let chits = search_code(&cidx, &q_code, 5, 0.0).await.unwrap();
    assert!(
        !mhits.is_empty(),
        "memory layer should return at least one hit"
    );
    assert!(
        !chits.is_empty(),
        "code layer should return at least one hit"
    );
    assert_eq!(
        mhits[0].id, rec.frontmatter.id,
        "closest memory hit should match the saved record"
    );
    assert!(
        chits[0].qualified.ends_with(":run_migration"),
        "closest code hit should be the indexed symbol, got: {}",
        chits[0].qualified
    );
}
