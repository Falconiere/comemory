//! Bench: vector-only baseline vs RRF-fused `search_memory_fused` over a
//! 100-row seeded corpus. Reports the latency delta directly.
//!
//! Three variants are tracked:
//!
//! - `search_vector_only` — dense-only baseline via
//!   `search_memory_fused_with_fts(idx, None, ...)`. Passing `None` for the
//!   FTS handle short-circuits the BM25 path so we measure pure dense
//!   retrieval through the unified entry point (there is no longer a
//!   separate `hybrid::search_memory`).
//! - `search_fused_rrf_cold_fts` — production code path: every iteration
//!   re-opens the FTS5 SQLite file (mirrors `comemory search` real-world).
//! - `search_fused_rrf_warm_fts` — fusion latency in isolation: the `Fts`
//!   handle is opened once outside the timed loop and reused.
//!
//! Heavy setup (embedder, LanceDB table, FTS handle, query embedding) is
//! built once before the criterion timer starts so the headline numbers
//! measure actual retrieval work rather than init cost.

use comemory::index::{Embedder, Fts, MemoryIndex};
use comemory::retrieval::fuse::{search_memory_fused, search_memory_fused_with_fts};
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
    // Warm FTS handle opened once outside the timed loop. The seed step
    // already created `fts.sqlite`, so this never short-circuits.
    let fts = Fts::open(paths.fts_db()).expect("fts");

    c.bench_function("search_vector_only", |b| {
        b.to_async(&rt).iter(|| async {
            search_memory_fused_with_fts(&idx, None, &paths, &q_vec, &q_text, 12, 0.0, 60.0)
                .await
                .expect("search dense-only")
        });
    });

    c.bench_function("search_fused_rrf_cold_fts", |b| {
        b.to_async(&rt).iter(|| async {
            search_memory_fused(&idx, &paths, &q_vec, &q_text, 12, 0.0, 60.0)
                .await
                .expect("fused")
        });
    });

    c.bench_function("search_fused_rrf_warm_fts", |b| {
        b.to_async(&rt).iter(|| async {
            search_memory_fused_with_fts(&idx, Some(&fts), &paths, &q_vec, &q_text, 12, 0.0, 60.0)
                .await
                .expect("fused warm")
        });
    });
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
