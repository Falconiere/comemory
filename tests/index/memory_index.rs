use qwick::config::paths::Paths;
use qwick::index::{Embedder, MemoryIndex};
use qwick::memory::{Kind, MemoryStore};

use super::common;

#[tokio::test]
async fn upsert_then_search_returns_hit() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save(
            "Use Postgres for analytics",
            Kind::Decision,
            "r",
            &[],
            "a",
            3,
        )
        .unwrap();

    let mut emb = Embedder::nomic_text().unwrap();
    let v = emb.embed_one(&rec.body).unwrap();
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    idx.upsert(&rec, &v).await.unwrap();

    let q = emb.embed_one("Postgres analytics decision").unwrap();
    let hits = idx.search(&q, 5).await.unwrap();
    assert!(!hits.is_empty(), "search returned no hits");
    assert_eq!(hits[0].id, rec.frontmatter.id);
    assert_eq!(hits[0].kind, Kind::Decision);
    assert_eq!(hits[0].repo, "r");
    assert!(
        hits[0].body.contains("Postgres"),
        "expected hit body to contain query subject, got: {}",
        hits[0].body
    );
}

#[tokio::test]
async fn search_on_empty_index_returns_empty() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    // Empty index: no embedder call needed; query with a zero vector and
    // expect the table-not-found short-circuit.
    let hits = idx.search(&vec![0.0_f32; 768], 5).await.unwrap();
    assert!(hits.is_empty());
}

#[tokio::test]
async fn upsert_same_id_overwrites_not_duplicates() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save("postgres for analytics", Kind::Decision, "r", &[], "a", 3)
        .unwrap();

    let mut emb = Embedder::nomic_text().unwrap();
    let v = emb.embed_one(&rec.body).unwrap();
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    idx.upsert(&rec, &v).await.unwrap();
    // Second upsert with the same id should merge-update, not insert a duplicate.
    idx.upsert(&rec, &v).await.unwrap();

    let hits = idx.search(&v, 10).await.unwrap();
    let same_id = hits.iter().filter(|h| h.id == rec.frontmatter.id).count();
    assert_eq!(same_id, 1, "expected exactly one row for id, got {same_id}");
}
