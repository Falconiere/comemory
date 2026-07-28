//! Mirror for `src/eval/tune_sample.rs` — the seeded candidate sampler:
//! determinism, pool membership, dedup/count, and the exhaustion fallback.

use comemory::config::TuneConfig;
use comemory::eval::tune::{self, TuneCandidate};
use comemory::eval::tune_sample::sample_candidates;

/// A 3^6 = 729-point surface (the shipped defaults) with `samples` set.
fn pools(samples: usize) -> TuneConfig {
    TuneConfig {
        samples,
        ..TuneConfig::default()
    }
}

/// Assert every candidate's value came from its configured pool.
fn assert_from_pools(t: &TuneConfig, drawn: &[TuneCandidate]) {
    for c in drawn {
        assert!(t.rrf_k_grid.contains(&c.rrf_k), "rrf_k off-pool: {c:?}");
        assert!(t.decay_grid.contains(&c.decay), "decay off-pool: {c:?}");
        assert!(
            t.mmr_lambda_grid.contains(&c.mmr_lambda),
            "mmr_lambda off-pool: {c:?}"
        );
        assert!(
            t.bm25_grid.contains(&c.bm25_weights),
            "bm25_weights off-pool: {c:?}"
        );
        assert!(
            t.graph_hops_grid.contains(&c.graph_hops),
            "graph_hops off-pool: {c:?}"
        );
        assert!(
            t.graph_seeds_grid.contains(&c.graph_seeds),
            "graph_seeds off-pool: {c:?}"
        );
    }
}

/// Assert no candidate repeats.
fn assert_unique(drawn: &[TuneCandidate]) {
    for (i, a) in drawn.iter().enumerate() {
        for b in &drawn[i + 1..] {
            assert_ne!(a, b, "sampled candidates must be distinct");
        }
    }
}

#[test]
fn sampling_is_deterministic_for_one_seed() {
    let t = pools(64);
    let first = sample_candidates(&t, 42);
    let second = sample_candidates(&t, 42);
    assert_eq!(first, second, "one seed must reproduce one draw sequence");
}

#[test]
fn different_seeds_draw_different_candidate_sets() {
    // 64 of 729 points: two seeds agreeing on the whole multiset would be
    // a ~1-in-C(729,64) coincidence, so inequality is a real signal.
    let t = pools(64);
    assert_ne!(sample_candidates(&t, 42), sample_candidates(&t, 7));
}

#[test]
fn every_sampled_candidate_is_a_grid_point() {
    let t = pools(64);
    let drawn = sample_candidates(&t, 42);
    assert_from_pools(&t, &drawn);
    let grid = tune::grid(&t);
    for c in &drawn {
        assert!(grid.contains(c), "sample must be a grid point: {c:?}");
    }
}

#[test]
fn sample_count_is_samples_capped_by_the_pool_product() {
    let t = pools(64);
    let drawn = sample_candidates(&t, 42);
    assert_eq!(
        drawn.len(),
        64,
        "the pool product (729) is not the binding cap"
    );
    assert_unique(&drawn);
}

#[test]
fn a_pool_smaller_than_the_budget_is_enumerated_exhaustively() {
    // 2 × 1 × 1 × 1 × 2 × 1 = 4 points but 50 requested: the draw loop
    // cannot find 50 distinct points, so the grid-order fallback tops the
    // quota up and the result is the whole grid, once.
    let t = TuneConfig {
        rrf_k_grid: vec![20.0, 60.0],
        decay_grid: vec![0.5],
        mmr_lambda_grid: vec![0.7],
        bm25_grid: vec![(1.0, 3.0)],
        graph_hops_grid: vec![0, 2],
        graph_seeds_grid: vec![8],
        samples: 50,
    };
    let drawn = sample_candidates(&t, 42);
    assert_eq!(drawn.len(), 4, "count is min(samples, pool_product)");
    assert_unique(&drawn);
    for c in tune::grid(&t) {
        assert!(drawn.contains(&c), "exhausted pool must yield every point");
    }
}

#[test]
fn a_singleton_surface_yields_its_one_point() {
    let t = TuneConfig {
        rrf_k_grid: vec![60.0],
        decay_grid: vec![0.5],
        mmr_lambda_grid: vec![0.7],
        bm25_grid: vec![(1.0, 3.0)],
        graph_hops_grid: vec![2],
        graph_seeds_grid: vec![8],
        samples: 64,
    };
    assert_eq!(sample_candidates(&t, 42), tune::grid(&t));
}

#[test]
fn an_empty_pool_yields_no_candidates() {
    // `Config::validate` rejects empty pools, so this is the sampler's own
    // guard: it must return empty rather than divide by a zero pool size.
    let t = TuneConfig {
        graph_hops_grid: vec![],
        ..pools(64)
    };
    assert!(sample_candidates(&t, 42).is_empty());
}
