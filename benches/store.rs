//! Store-primitive micro-benches over a ≥1k-row corpus with vectors seeded:
//! vec0 KNN (`knn_memory` / `knn_code`), FTS5 BM25 (`search_memory` /
//! `search_code`), and the recursive-CTE supersedes walk
//! (`supersedes_chain`). The corpus (and the edge chain) is built once
//! outside the timer; KNN is asserted non-empty so a regression to a
//! zero-row query fails loudly instead of reporting ~0ns.

#[path = "common/corpus.rs"]
mod corpus;

use corpus::vectors::vector;
use corpus::{BenchCorpus, CODE_DIM, MEMORY_DIM, build_corpus};

use comemory::graph::edges::{self, EdgeKey};
use comemory::store::{fts, vector as store_vec};
use criterion::{Criterion, criterion_group, criterion_main};

/// Rows seeded for the store micro-benches (≥1k per acceptance #4).
const ROWS: usize = 2_000;
/// Top-k requested by the KNN / FTS benches.
const K: usize = 12;

/// Build the corpus once and link consecutive memories into a supersedes
/// chain so `supersedes_chain` walks a real recursive CTE.
fn setup() -> BenchCorpus {
    let corpus = build_corpus(ROWS, ROWS);
    for pair in corpus.mem_ids.windows(2) {
        edges::insert(
            &corpus.conn,
            EdgeKey {
                src_kind: "memory",
                src_id: &pair[1],
                dst_kind: "memory",
                dst_id: &pair[0],
                rel: "supersedes",
            },
        )
        .unwrap();
    }
    corpus
}

/// Time vec0 KNN over the seeded memory and code vectors.
fn bench_knn(c: &mut Criterion) {
    let corpus = setup();
    let mvec = vector("q", MEMORY_DIM);
    let cvec = vector("q", CODE_DIM);
    assert!(
        !store_vec::knn_memory(&corpus.conn, &mvec, K, None)
            .unwrap()
            .is_empty(),
        "memory KNN must return hits over a seeded corpus"
    );
    let mut group = c.benchmark_group("store/knn");
    group.bench_function("memory", |b| {
        b.iter(|| {
            std::hint::black_box(store_vec::knn_memory(&corpus.conn, &mvec, K, None).unwrap());
        });
    });
    group.bench_function("code", |b| {
        b.iter(|| {
            std::hint::black_box(store_vec::knn_code(&corpus.conn, &cvec, K, None, None).unwrap());
        });
    });
    group.finish();
}

/// Time FTS5 BM25 over the seeded memory and code FTS rows.
fn bench_fts(c: &mut Criterion) {
    let corpus = setup();
    let mut group = c.benchmark_group("store/fts");
    group.bench_function("memory", |b| {
        b.iter(|| {
            let hits = fts::search_memory(
                &corpus.conn,
                "postgres pool ranking",
                K,
                None,
                None,
                corpus.cfg.retrieval.bm25_weights,
            )
            .unwrap();
            std::hint::black_box(hits.len());
        });
    });
    group.bench_function("code", |b| {
        b.iter(|| {
            let hits = fts::search_code(
                &corpus.conn,
                "postgres compute vector",
                K,
                None,
                None,
                corpus.cfg.retrieval.code_bm25_weights,
            )
            .unwrap();
            std::hint::black_box(hits.len());
        });
    });
    group.finish();
}

/// Time the recursive-CTE supersedes walk from the chain head.
fn bench_supersedes_chain(c: &mut Criterion) {
    let corpus = setup();
    let start = corpus.mem_ids.last().unwrap().clone();
    let code_count = corpus.code_ids.len();
    assert!(code_count >= ROWS, "code corpus seeded");
    c.bench_function("store/supersedes_chain", |b| {
        b.iter(|| {
            let chain = edges::supersedes_chain(&corpus.conn, &start, 64).unwrap();
            std::hint::black_box(chain.len());
        });
    });
}

criterion_group!(benches, bench_knn, bench_fts, bench_supersedes_chain);
criterion_main!(benches);
