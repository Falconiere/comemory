//! Graph-layer benches: pure kernels (`pagerank` over a synthetic edge
//! list, `extract_imports` + `PathIndex::resolve` over real source) and the
//! git-backed miners (`mine_cochange`, `materialize`) over a real on-disk
//! repo built once at setup. Heavy fixtures (the git repo, the seeded
//! `code_symbols`) live outside `b.iter`.

#[path = "common/gitrepo.rs"]
mod gitrepo;

use std::collections::HashSet;

use gitrepo::{build_git_repo, seed_repo_symbols};

use comemory::ast::languages::Lang;
use comemory::graph::imports::{PathIndex, extract_imports};
use comemory::graph::{cochange, materialize, pagerank};
use comemory::store::connection;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

/// Node-count sweep for the pagerank kernel.
const PAGERANK_SWEEP: [usize; 3] = [100, 1_000, 10_000];
/// Commits in the git fixture (≥50 so co-change pairs are non-trivial).
const COMMITS: usize = 60;

/// A real comemory source file with imports, embedded at compile time.
const IMPORTS_SRC: &str = include_str!("../src/cli/mod.rs");

/// Deterministic edge list over `n` nodes: each node points at `(i*7+3) % n`.
fn synthetic_edges(n: usize) -> Vec<(u32, u32, f64)> {
    (0..n)
        .map(|i| (i as u32, ((i * 7 + 3) % n) as u32, 1.0))
        .collect()
}

/// Time `pagerank` over the synthetic edge list across the node sweep.
fn bench_pagerank(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph/pagerank");
    for &n in &PAGERANK_SWEEP {
        let edges = synthetic_edges(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| std::hint::black_box(pagerank::pagerank(n, &edges)));
        });
    }
    group.finish();
}

/// Time `extract_imports` and `PathIndex::resolve` over real source.
fn bench_imports(c: &mut Criterion) {
    let modules = extract_imports(Lang::Rust, IMPORTS_SRC).unwrap();
    let indexed: Vec<String> = (0..200).map(|i| format!("src/mod{i}/file{i}.rs")).collect();
    let index = PathIndex::new(&indexed);
    let mut group = c.benchmark_group("graph/imports");
    group.bench_function("extract", |b| {
        b.iter(|| {
            let out = extract_imports(Lang::Rust, IMPORTS_SRC).unwrap();
            std::hint::black_box(out.len());
        });
    });
    group.bench_function("resolve", |b| {
        b.iter(|| {
            for m in &modules {
                std::hint::black_box(index.resolve(m, Some("src/mod0/file0.rs")));
            }
        });
    });
    group.finish();
}

/// Time `mine_cochange` and `materialize` over a real git repo.
fn bench_cochange(c: &mut Criterion) {
    let (tmp, root, known) = build_git_repo(COMMITS);
    let known: HashSet<String> = known;
    let out = cochange::mine_cochange(&root, &known, None).unwrap();
    assert!(!out.pairs.is_empty(), "≥2 files/commit must yield pairs");

    let mut group = c.benchmark_group("graph/cochange");
    group.bench_function("mine", |b| {
        b.iter(|| {
            let o = cochange::mine_cochange(&root, &known, None).unwrap();
            std::hint::black_box(o.pairs.len());
        });
    });

    let mut conn = connection::open(tmp.path().join("comemory.db")).unwrap();
    let imports = seed_repo_symbols(&conn, "bench", &root);
    group.bench_function("materialize", |b| {
        b.iter(|| {
            materialize::materialize(&mut conn, &root, "bench", &imports).unwrap();
        });
    });
    group.finish();
}

criterion_group!(benches, bench_pagerank, bench_imports, bench_cochange);
criterion_main!(benches);
