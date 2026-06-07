//! Bench: end-to-end `comemory save` cost (markdown write + nomic embedding +
//! `MemoryIndex::upsert` + FTS5 insert). Reports mean + p99 in nanoseconds
//! via criterion.
//!
//! Heavy setup (fixture, embedder, LanceDB connection, FTS handle) is
//! constructed once outside the criterion timer. Each iteration varies
//! the body via an atomic counter so the resulting `memory_id` (sha256 of
//! body) is unique per iteration: we measure the cold-insert path across
//! all three writers (markdown create, dense `merge_insert` against an
//! id never seen before, FTS row insert), not the steady-state overwrite.

use std::sync::atomic::{AtomicUsize, Ordering};

use comemory::index::{Embedder, Fts, MemoryIndex};
use comemory::memory::{Kind, MemoryStore};
use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;
use tokio::sync::Mutex;

mod common;

/// Per-iteration counter so each bench iteration writes a new id. Atomic
/// so the counter is cheap and future-proof against criterion threading
/// changes; today the harness is single-threaded.
static BENCH_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn bench_save(c: &mut Criterion) {
    let rt = Runtime::new().expect("rt");
    let fx = common::fixture();
    let emb = Mutex::new(Embedder::nomic_text().expect("emb"));
    let idx = rt
        .block_on(MemoryIndex::open(fx.paths.vectors_dir(), 768))
        .expect("idx");
    let fts = Fts::open(fx.paths.fts_db()).expect("fts");
    let store = MemoryStore::new(fx.paths.clone());

    c.bench_function("save_end_to_end", |b| {
        b.to_async(&rt).iter(|| async {
            let n = BENCH_COUNTER.fetch_add(1, Ordering::Relaxed);
            let body = format!("bench body {n}: postgres analytics decision token");
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
