use qwick::config::paths::Paths;
use qwick::index::{CodeIndex, Embedder, MemoryIndex};
use qwick::memory::{Kind, MemoryStore};
use qwick::retrieval::hybrid::{search_code, search_memory};

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
    let mhits = search_memory(&midx, &q_text, 5, 0.0).await.unwrap();
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
