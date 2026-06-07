//! Bench: end-to-end `comemory save` cost (markdown write + nomic embedding +
//! `MemoryIndex::upsert` + FTS5 insert). Reports mean + p99 in nanoseconds
//! via criterion.
//!
//! Heavy setup (fixture, embedder, LanceDB connection, FTS handle) is
//! constructed once outside the criterion timer. Each iteration overwrites
//! the same memory id, so we measure the steady-state upsert path
//! (`merge_insert` on dense, `DELETE`+`INSERT` on FTS) rather than init.

use comemory::index::{Embedder, Fts, MemoryIndex};
use comemory::memory::{Kind, MemoryStore};
use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;
use tokio::sync::Mutex;

mod common;

fn bench_save(c: &mut Criterion) {
    let rt = Runtime::new().expect("rt");
    let fx = common::fixture();
    let emb = Mutex::new(Embedder::nomic_text().expect("emb"));
    let idx = rt
        .block_on(MemoryIndex::open(fx.paths.vectors_dir(), 768))
        .expect("idx");
    let fts = Fts::open(fx.paths.fts_db()).expect("fts");
    let store = MemoryStore::new(fx.paths.clone());
    let body = String::from("Bench memory: postgres analytics decision token");

    c.bench_function("save_end_to_end", |b| {
        b.to_async(&rt).iter(|| async {
            let rec = store
                .save(&body, Kind::Note, "bench", &[], "bench", 3)
                .expect("save");
            let v = {
                let mut guard = emb.lock().await;
                guard.embed_one(&rec.body).expect("embed")
            };
            idx.upsert(&rec, &v).await.expect("upsert");
            fts.upsert(&rec.frontmatter.id, &rec.body)
                .expect("fts upsert");
        });
    });
}

criterion_group!(benches, bench_save);
criterion_main!(benches);
