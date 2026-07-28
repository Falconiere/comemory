//! Seeded uniform sampling over the `[tune]` grid pools.
//!
//! Split out of [`crate::eval::tune`] (Binding Rule 3): `tune.rs` owns the
//! exhaustive grid, scoring, and the config.toml apply; this module owns
//! candidate *selection* — the draw loop and its seed derivation.

use std::collections::HashSet;

use sha2::{Digest, Sha256};

use crate::config::TuneConfig;
use crate::eval::bandit_rng::SplitMix64;
use crate::eval::tune::TuneCandidate;

/// Draw budget per requested sample before the sampler stops drawing and
/// tops the quota up by enumeration. Bounds the loop when the pools are
/// nearly exhausted (drawing the last unseen point of a small product can
/// otherwise take unboundedly many attempts).
const MAX_DRAW_FACTOR: usize = 16;

/// Pool sizes in [`crate::eval::tune::grid`] nesting order: `graph_hops`,
/// `graph_seeds`, `rrf_k`, `decay`, `mmr_lambda`, `bm25`. Index 0 is the
/// outermost (most significant) dimension, so a mixed-radix counter over
/// these sizes walks the grid in exactly `grid()`'s order.
pub(crate) fn pool_sizes(t: &TuneConfig) -> [usize; 6] {
    [
        t.graph_hops_grid.len(),
        t.graph_seeds_grid.len(),
        t.rrf_k_grid.len(),
        t.decay_grid.len(),
        t.mmr_lambda_grid.len(),
        t.bm25_grid.len(),
    ]
}

/// Materialize the candidate at mixed-radix `idx` (dimension order per
/// [`pool_sizes`]). `None` only when an index is out of range, which the
/// empty-pool guard in [`sample_candidates`] makes unreachable.
fn candidate_at(t: &TuneConfig, idx: [usize; 6]) -> Option<TuneCandidate> {
    Some(TuneCandidate {
        graph_hops: *t.graph_hops_grid.get(idx[0])?,
        graph_seeds: *t.graph_seeds_grid.get(idx[1])?,
        rrf_k: *t.rrf_k_grid.get(idx[2])?,
        decay: *t.decay_grid.get(idx[3])?,
        mmr_lambda: *t.mmr_lambda_grid.get(idx[4])?,
        bm25_weights: *t.bm25_grid.get(idx[5])?,
    })
}

/// Record `idx` and push its candidate when this grid point is new.
/// Shared by both fill paths (Binding Rule 1); the `Some` arm is written
/// out rather than folded into `Vec::extend` so the unreachable-`None`
/// contract of [`candidate_at`] stays visible at the call site.
fn push_unseen(
    t: &TuneConfig,
    idx: [usize; 6],
    seen: &mut HashSet<[usize; 6]>,
    out: &mut Vec<TuneCandidate>,
) {
    if seen.insert(idx)
        && let Some(candidate) = candidate_at(t, idx)
    {
        out.push(candidate);
    }
}

/// Mixed-radix decomposition of `linear` over `sizes`, index 0 most
/// significant. `None` once `linear` reaches the pool product (or a pool
/// is empty), which ends the enumeration walk.
fn decompose(linear: usize, sizes: &[usize; 6]) -> Option<[usize; 6]> {
    let mut rest = linear;
    let mut idx = [0usize; 6];
    for (slot, &n) in idx.iter_mut().zip(sizes.iter()).rev() {
        if n == 0 {
            return None;
        }
        *slot = rest % n;
        rest /= n;
    }
    if rest == 0 { Some(idx) } else { None }
}

/// The product of the pool sizes, saturating instead of overflowing.
fn pool_product(sizes: &[usize; 6]) -> usize {
    sizes
        .iter()
        .try_fold(1usize, |acc, &n| acc.checked_mul(n))
        .unwrap_or(usize::MAX)
}

/// Deterministic top-up: walk the pools in `grid()` order and take the
/// first still-unseen points until the quota is met or the grid runs out.
fn fill_in_grid_order(
    t: &TuneConfig,
    sizes: &[usize; 6],
    seen: &mut HashSet<[usize; 6]>,
    out: &mut Vec<TuneCandidate>,
    target: usize,
) {
    let mut linear = 0usize;
    while out.len() < target {
        let Some(idx) = decompose(linear, sizes) else {
            return;
        };
        linear += 1;
        push_unseen(t, idx, seen, out);
    }
}

/// Sample `min(t.samples, pool_product)` distinct grid points, one uniform
/// draw per dimension per candidate, deterministic in `seed`. After
/// `t.samples × 16` draws the quota is topped up by grid-order enumeration,
/// so nearly-exhausted pools cannot spin. An empty pool (rejected by
/// `Config::validate`) yields nothing.
///
/// Trade-off: indices come from `next_u64() % pool.len()`, whose modulo
/// bias at pool sizes of a few dozen is below 2⁻⁵⁵ — far under a golden
/// score's noise floor, so rejection sampling would buy only branches.
pub fn sample_candidates(t: &TuneConfig, seed: u64) -> Vec<TuneCandidate> {
    let sizes = pool_sizes(t);
    if sizes.contains(&0) {
        return Vec::new();
    }
    let target = t.samples.min(pool_product(&sizes));
    let max_draws = t.samples.saturating_mul(MAX_DRAW_FACTOR);
    let mut rng = SplitMix64::new(seed);
    let mut seen: HashSet<[usize; 6]> = HashSet::with_capacity(target);
    let mut out = Vec::with_capacity(target);
    let mut draws = 0usize;
    while out.len() < target && draws < max_draws {
        draws += 1;
        let mut idx = [0usize; 6];
        for (slot, &n) in idx.iter_mut().zip(sizes.iter()) {
            *slot = (rng.next_u64() % n as u64) as usize;
        }
        push_unseen(t, idx, &mut seen, &mut out);
    }
    if out.len() < target {
        fill_in_grid_order(t, &sizes, &mut seen, &mut out, target);
    }
    out
}

/// Deterministic default seed from golden-set size + pool shape + schema
/// version. Same construction as [`crate::eval::bandit::sample_seed`],
/// which hashes an arm count where this hashes the whole pool shape:
/// unchanged inputs reproduce a run bit-identically with no `--seed`,
/// while widening a grid or the corpus reshuffles the draw.
///
/// `pool_sizes` is a slice, not `&[usize; 6]`, so the hash is defined for
/// any pool shape; the single caller in [`crate::eval::tune`] passes the
/// six-element array from [`pool_sizes`].
pub(crate) fn sample_seed(pairs: usize, pool_sizes: &[usize]) -> u64 {
    let mut h = Sha256::new();
    h.update(pairs.to_le_bytes());
    for &n in pool_sizes {
        h.update(n.to_le_bytes());
    }
    h.update(crate::store::migrate::CURRENT_VERSION.as_bytes());
    let dig = h.finalize();
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&dig[..8]);
    u64::from_le_bytes(buf)
}
