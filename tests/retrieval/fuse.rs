use comemory::config::paths::Paths;
use comemory::index::{Embedder, Fts, MemoryIndex};
use comemory::memory::{Kind, MemoryStore};
use comemory::retrieval::fuse::{search_memory_fused, FuseOptions};

use super::common;

/// Common bench/test default: no dense threshold, RRF 60. Limit is set per
/// test via `..DEFAULT_OPTS`.
const DEFAULT_OPTS: FuseOptions = FuseOptions {
    limit: 5,
    dense_threshold: 0.0,
    rrf_k: 60.0,
};

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
    let hits = search_memory_fused(&idx, &paths, &q, "zzzyx_unique_token", DEFAULT_OPTS)
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
    let hits = search_memory_fused(&idx, &paths, &q, "postgres analytics", DEFAULT_OPTS)
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
    let hits = search_memory_fused(
        &idx,
        &paths,
        &q,
        "any body text",
        FuseOptions {
            limit: 0,
            ..DEFAULT_OPTS
        },
    )
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
    let hits = search_memory_fused(
        &idx,
        &paths,
        &q,
        "zzzyx_unique_token",
        FuseOptions {
            limit,
            ..DEFAULT_OPTS
        },
    )
    .await
    .unwrap();

    assert!(
        hits.iter().any(|h| h.id == rare.frontmatter.id),
        "fused search dropped sparse-only hit outside dense over-fetch window: got {:?}",
        hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>()
    );
}

/// Regression for G2: `dense_threshold` must prune weak-cosine dense
/// candidates *before* they feed into RRF. A memory whose embedding is
/// semantically far from the query (low cosine) must be excluded from the
/// dense side of the fused list when a non-zero threshold is supplied,
/// while a memory whose body matches the query lexically (BM25-strong) must
/// still survive via the sparse path.
///
/// Mechanism check: we set the threshold to `1.5` (impossible — the cosine
/// score is bounded by `1.0`) so every dense candidate is pruned. The
/// fused result must come entirely from the BM25 path. Then we re-run with
/// threshold `0.0` and assert the dense hits are now visible.
#[tokio::test]
async fn fused_search_applies_dense_threshold_before_fusion() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    let store = MemoryStore::new(paths.clone());
    // Memory A: rare token in body, BM25 will lift it on a rare-token query.
    let rare = store
        .save(
            "An essay containing the arcane phrase zzzyx_unique_token only",
            Kind::Note,
            "r",
            &[],
            "a",
            3,
        )
        .unwrap();
    // Memory B: no rare token, only a dense candidate.
    let dense = store
        .save(
            "Postgres analytics dashboard rollout decision",
            Kind::Decision,
            "r",
            &[],
            "a",
            3,
        )
        .unwrap();

    let mut emb = Embedder::nomic_text().unwrap();
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    let v_rare = emb.embed_one(&rare.body).unwrap();
    idx.upsert(&rare, &v_rare).await.unwrap();
    let v_dense = emb.embed_one(&dense.body).unwrap();
    idx.upsert(&dense, &v_dense).await.unwrap();

    let fts = Fts::open(paths.fts_db()).unwrap();
    fts.upsert(&rare.frontmatter.id, &rare.body).unwrap();
    fts.upsert(&dense.frontmatter.id, &dense.body).unwrap();
    drop(fts);

    // Query that matches the rare token via BM25. The dense path will return
    // both rows; with threshold 1.5 (above the 1.0 cosine ceiling), the
    // dense side is fully pruned and only BM25 contributes.
    let q = emb.embed_one("zzzyx_unique_token").unwrap();
    let hits_strict = search_memory_fused(
        &idx,
        &paths,
        &q,
        "zzzyx_unique_token",
        FuseOptions {
            dense_threshold: 1.5,
            ..DEFAULT_OPTS
        },
    )
    .await
    .unwrap();
    // With dense pruned, only the BM25-matched id (`rare`) should appear.
    assert!(
        hits_strict.iter().any(|h| h.id == rare.frontmatter.id),
        "BM25-strong rare-token id must survive dense pruning, got {:?}",
        hits_strict
            .iter()
            .map(|h| h.id.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        !hits_strict.iter().any(|h| h.id == dense.frontmatter.id),
        "dense-only id must be pruned when threshold > 1.0, got {:?}",
        hits_strict
            .iter()
            .map(|h| h.id.as_str())
            .collect::<Vec<_>>()
    );

    // With threshold 0.0, both ids should reappear (dense path unfiltered).
    let hits_open = search_memory_fused(&idx, &paths, &q, "zzzyx_unique_token", DEFAULT_OPTS)
        .await
        .unwrap();
    assert!(
        hits_open.iter().any(|h| h.id == dense.frontmatter.id),
        "dense-only id must reappear with threshold 0.0, got {:?}",
        hits_open.iter().map(|h| h.id.as_str()).collect::<Vec<_>>()
    );
}
