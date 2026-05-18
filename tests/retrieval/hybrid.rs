use qwick_memory::config::paths::Paths;
use qwick_memory::index::{Embedder, MemoryIndex};
use qwick_memory::memory::{Kind, MemoryStore};
use qwick_memory::retrieval::hybrid::search_memory;

use super::common;

#[tokio::test]
async fn search_memory_filters_below_threshold() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save(
            "Postgres analytics decision",
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
    let hits_pass = search_memory(&idx, &q, 5, 0.0).await.unwrap();
    assert!(!hits_pass.is_empty(), "threshold 0.0 should keep hits");

    let hits_filtered = search_memory(&idx, &q, 5, 1.5).await.unwrap();
    assert!(
        hits_filtered.is_empty(),
        "absurd threshold should empty results"
    );
}

#[tokio::test]
async fn search_memory_returns_sorted_descending() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());

    let a = store
        .save(
            "Postgres analytics decision",
            Kind::Decision,
            "r",
            &[],
            "a",
            3,
        )
        .unwrap();
    let b = store
        .save("Totally unrelated topic", Kind::Note, "r", &[], "a", 3)
        .unwrap();

    let mut emb = Embedder::nomic_text().unwrap();
    let va = emb.embed_one(&a.body).unwrap();
    let vb = emb.embed_one(&b.body).unwrap();
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    idx.upsert(&a, &va).await.unwrap();
    idx.upsert(&b, &vb).await.unwrap();

    let q = emb.embed_one("Postgres analytics decision").unwrap();
    let hits = search_memory(&idx, &q, 5, 0.0).await.unwrap();
    assert!(hits.len() >= 2, "expected both memories returned");
    for w in hits.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "hits should be sorted desc by score: {} then {}",
            w[0].score,
            w[1].score
        );
    }
    assert_eq!(
        hits[0].id, a.frontmatter.id,
        "closest match should rank first"
    );
}

#[tokio::test]
async fn search_memory_respects_limit() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let mut emb = Embedder::nomic_text().unwrap();
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();

    for i in 0..4 {
        let body = format!("memory body number {i}");
        let rec = store.save(&body, Kind::Note, "r", &[], "a", 3).unwrap();
        let v = emb.embed_one(&rec.body).unwrap();
        idx.upsert(&rec, &v).await.unwrap();
    }

    let q = emb.embed_one("memory body").unwrap();
    let hits = search_memory(&idx, &q, 2, 0.0).await.unwrap();
    assert_eq!(hits.len(), 2, "limit must be honoured after sort/filter");
}
