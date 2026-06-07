use comemory::config::paths::Paths;
use comemory::index::{Embedder, Fts, MemoryIndex};
use comemory::memory::{Kind, MemoryStore};
use comemory::retrieval::fuse::search_memory_fused;

use super::common;

#[tokio::test]
async fn fused_search_finds_lexical_only_match() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save(
            "The arcane phrase zzzyx_unique_token only appears here",
            Kind::Note,
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

    let fts = Fts::open(paths.index_dir().join("fts.sqlite")).unwrap();
    fts.upsert(&rec.frontmatter.id, &rec.body).unwrap();
    drop(fts);

    let q = emb.embed_one("zzzyx_unique_token").unwrap();
    let hits = search_memory_fused(
        &idx,
        &paths.index_dir().join("fts.sqlite"),
        &q,
        "zzzyx_unique_token",
        5,
        60.0,
    )
    .await
    .unwrap();
    assert!(
        hits.iter().any(|h| h.id == rec.frontmatter.id),
        "fused search dropped the lexical-only match"
    );
}

#[tokio::test]
async fn fused_search_degrades_to_vector_when_fts_missing() {
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

    let q = emb.embed_one("postgres analytics").unwrap();
    let hits = search_memory_fused(
        &idx,
        &paths.index_dir().join("missing.sqlite"),
        &q,
        "postgres analytics",
        5,
        60.0,
    )
    .await
    .unwrap();
    assert_eq!(hits[0].id, rec.frontmatter.id);
}
