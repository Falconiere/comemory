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

    let fts = Fts::open(paths.fts_db()).unwrap();
    fts.upsert(&rec.frontmatter.id, &rec.body).unwrap();
    drop(fts);

    let q = emb.embed_one("zzzyx_unique_token").unwrap();
    let hits = search_memory_fused(&idx, &paths, &q, "zzzyx_unique_token", 5, 60.0)
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
    // Delete the FTS db so the path doesn't exist; sparse path degrades.
    let fts_db = paths.fts_db();
    if fts_db.exists() {
        std::fs::remove_file(&fts_db).unwrap();
    }
    let hits = search_memory_fused(&idx, &paths, &q, "postgres analytics", 5, 60.0)
        .await
        .unwrap();
    assert_eq!(hits[0].id, rec.frontmatter.id);
}

/// Regression for C2: `limit == 0` must short-circuit to an empty vec
/// without exercising the dense or sparse path (the dense path is a
/// `Result<…, lancedb::Error>` whose `limit(0)` behaviour is undefined).
#[tokio::test]
async fn fused_search_limit_zero_returns_empty() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save("any body text", Kind::Note, "r", &[], "a", 3)
        .unwrap();
    let mut emb = Embedder::nomic_text().unwrap();
    let v = emb.embed_one(&rec.body).unwrap();
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    idx.upsert(&rec, &v).await.unwrap();

    let q = emb.embed_one("any body text").unwrap();
    let hits = search_memory_fused(&idx, &paths, &q, "any body text", 0, 60.0)
        .await
        .unwrap();
    assert!(hits.is_empty());
}

/// Regression for C1: when an id is FTS-rank-1 but pushed out of the dense
/// over-fetch window, the fused search must still surface it by loading the
/// record from the markdown store.
#[tokio::test]
async fn fused_search_materializes_sparse_only_hit_outside_dense_window() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    let store = MemoryStore::new(paths.clone());
    let mut emb = Embedder::nomic_text().unwrap();
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    let fts = Fts::open(paths.fts_db()).unwrap();

    // Over-fetch window is limit * 4. With limit = 2 the dense pool is 8.
    // Seed `over * 2 + 1 = 17` decoy memories whose bodies are semantically
    // close to the query so they dominate the dense ranking and crowd out
    // the rare-token memory.
    let limit: usize = 2;
    let decoys = limit * 4 * 2 + 1;
    for i in 0..decoys {
        let body = format!("Use Postgres for analytics workload {i} dashboards reports");
        let rec = store.save(&body, Kind::Decision, "r", &[], "a", 3).unwrap();
        let v = emb.embed_one(&rec.body).unwrap();
        idx.upsert(&rec, &v).await.unwrap();
        fts.upsert(&rec.frontmatter.id, &rec.body).unwrap();
    }

    // The rare-token memory: semantically unrelated to "postgres analytics"
    // (so it falls outside the dense top-K) but unique on a token only it
    // matches.
    let rare = store
        .save(
            "Completely unrelated essay about zzzyx_unique_token alone here",
            Kind::Note,
            "r",
            &[],
            "a",
            3,
        )
        .unwrap();
    let v = emb.embed_one(&rare.body).unwrap();
    idx.upsert(&rare, &v).await.unwrap();
    fts.upsert(&rare.frontmatter.id, &rare.body).unwrap();
    drop(fts);

    // Query that matches the rare token via BM25 and the decoys via dense.
    let q = emb
        .embed_one("postgres analytics zzzyx_unique_token")
        .unwrap();
    let hits = search_memory_fused(&idx, &paths, &q, "zzzyx_unique_token", limit, 60.0)
        .await
        .unwrap();

    assert!(
        hits.iter().any(|h| h.id == rare.frontmatter.id),
        "fused search dropped sparse-only hit outside dense over-fetch window: got {:?}",
        hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>()
    );
}
