//! Retrieval hot-path benches: the full memory pipeline (`pipeline::search`)
//! and the code route (`route_code` -> `rerank_code`), swept over corpus
//! sizes 100 / 1 000 / 10 000. The corpus is built once per size outside the
//! criterion timer; only the query runs inside `b.iter`.

#[path = "common/corpus.rs"]
mod corpus;

use corpus::vectors::vector;
use corpus::{BenchCorpus, CODE_DIM, MEMORY_DIM, build_corpus};

use comemory::retrieval::pipeline::{self, PageWindow, SearchOptions};
use comemory::retrieval::{code_rerank, code_route};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

/// Corpus sizes swept by every sizing bench.
const SWEEP: [usize; 3] = [100, 1_000, 10_000];

/// Time `pipeline::search` (hybrid: lexical + ANN) across the sweep.
fn bench_memory_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("retrieval/memory_search");
    let qvec = vector("query terms", MEMORY_DIM);
    for &n in &SWEEP {
        let corpus = build_corpus(n, 0);
        assert_eq!(corpus.mem_ids.len(), n, "memory corpus seeded");
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| run_memory_search(&corpus, &qvec));
        });
    }
    group.finish();
}

/// One memory search over the prebuilt corpus — the timed region.
fn run_memory_search(corpus: &BenchCorpus, qvec: &[f32]) {
    let opts = SearchOptions {
        track: false,
        source: "search",
        window: PageWindow::top_k(&corpus.cfg),
    };
    let run = pipeline::search(
        &corpus.cfg,
        &corpus.conn,
        "postgres pool tokenizer ranking",
        Some(qvec),
        Some("bench"),
        None,
        opts,
    )
    .unwrap();
    std::hint::black_box(run.hits.len());
}

/// Time the code route + rerank across the sweep.
fn bench_code_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("retrieval/code_search");
    let qvec = vector("q", CODE_DIM);
    for &n in &SWEEP {
        let corpus = build_corpus(0, n);
        assert_eq!(corpus.code_ids.len(), n, "code corpus seeded");
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| run_code_search(&corpus, &qvec));
        });
    }
    group.finish();
}

/// One code route + rerank over the prebuilt corpus — the timed region.
fn run_code_search(corpus: &BenchCorpus, qvec: &[f32]) {
    let pool = pipeline::pool_size(
        0,
        corpus.cfg.retrieval.top_k,
        corpus.cfg.retrieval.max_page_window,
    );
    let hits = code_route::route_code(
        &corpus.cfg,
        &corpus.conn,
        "postgres compute vector",
        Some(qvec),
        Some("bench"),
        None,
        pool,
    )
    .unwrap();
    let ws = code_rerank::WorkingSet::default();
    let ranked = code_rerank::rerank_code(&corpus.conn, &corpus.cfg, &hits, &ws).unwrap();
    std::hint::black_box(ranked.len());
}

criterion_group!(benches, bench_memory_search, bench_code_search);
criterion_main!(benches);
