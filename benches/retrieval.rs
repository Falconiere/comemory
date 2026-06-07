//! Bench: vector-only `search_memory` vs RRF-fused `search_memory_fused` over
//! a 100-row seeded corpus. Reports the latency delta directly.

use comemory::index::{Embedder, MemoryIndex};
use comemory::retrieval::fuse::search_memory_fused;
use comemory::retrieval::hybrid::search_memory;
use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;

mod common;

fn bench_search(c: &mut Criterion) {
    let rt = Runtime::new().expect("rt");
    let fx = common::fixture();
    rt.block_on(async {
        let _ = common::seed(&fx.paths, 100).await;
    });
    let mut emb = Embedder::nomic_text().expect("emb");
    let q_vec = emb.embed_one("postgres analytics").expect("embed");
    let q_text = "postgres analytics".to_string();
    let idx = rt
        .block_on(MemoryIndex::open(fx.paths.vectors_dir(), 768))
        .expect("idx");
    let paths = fx.paths.clone();

    c.bench_function("search_vector_only", |b| {
        b.to_async(&rt)
            .iter(|| async { search_memory(&idx, &q_vec, 12, 0.55).await.expect("search") });
    });

    c.bench_function("search_fused_rrf", |b| {
        b.to_async(&rt).iter(|| async {
            search_memory_fused(&idx, &paths, &q_vec, &q_text, 12, 60.0)
                .await
                .expect("fused")
        });
    });
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
