use std::sync::Arc;

use arrow_array::{RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use comemory::config::paths::Paths;
use comemory::index::memory_index::collect_hits;
use comemory::index::{Embedder, MemoryHit, MemoryIndex};
use comemory::memory::{Kind, MemoryStore};

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

#[test]
fn collect_hits_errors_on_missing_distance() {
    // Build a `RecordBatch` matching the columns `collect_hits` expects, but
    // deliberately omit `_distance`. The decoder MUST surface the schema
    // mismatch so callers don't silently get every hit at score 1.0.
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("body", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("repo", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["abc"])),
            Arc::new(StringArray::from(vec!["body"])),
            Arc::new(StringArray::from(vec!["decision"])),
            Arc::new(StringArray::from(vec!["r"])),
        ],
    )
    .expect("build batch without _distance");

    let mut out: Vec<MemoryHit> = Vec::new();
    let err = collect_hits(&batch, &mut out).expect_err("missing _distance must error");
    let msg = err.to_string();
    assert!(
        msg.contains("missing _distance"),
        "error must mention missing _distance column, got: {msg}",
    );
    assert!(out.is_empty(), "no hits should be appended on error");
}
