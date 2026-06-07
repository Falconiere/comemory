//! Bench: end-to-end `comemory save` cost (markdown write + dense embed +
//! FTS insert). Reports mean + p99 in nanoseconds via criterion.

use comemory::index::{Embedder, Fts, MemoryIndex};
use comemory::memory::{Kind, MemoryStore};
use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;

mod common;

fn bench_save(c: &mut Criterion) {
    let rt = Runtime::new().expect("rt");
    c.bench_function("save_end_to_end", |b| {
        b.to_async(&rt).iter_with_setup(
            || -> (common::Fixture, String) {
                let fx = common::fixture();
                let body = String::from("Bench memory: postgres analytics decision token");
                (fx, body)
            },
            |pair: (common::Fixture, String)| async move {
                let (fx, body) = pair;
                let store = MemoryStore::new(fx.paths.clone());
                let rec = store
                    .save(&body, Kind::Note, "bench", &[], "bench", 3)
                    .expect("save");
                let idx = MemoryIndex::open(fx.paths.vectors_dir(), 768)
                    .await
                    .expect("idx");
                let mut emb = Embedder::nomic_text().expect("emb");
                let v = emb.embed_one(&rec.body).expect("embed");
                idx.upsert(&rec, &v).await.expect("upsert");
                let fts = Fts::open(fx.paths.index_dir().join("fts.sqlite")).expect("fts");
                fts.upsert(&rec.frontmatter.id, &rec.body)
                    .expect("fts upsert");
            },
        );
    });
}

criterion_group!(benches, bench_save);
criterion_main!(benches);
