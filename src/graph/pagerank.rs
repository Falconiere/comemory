//! Deterministic PageRank over a weighted directed graph, used to
//! materialize per-file importance into `code_symbols.rank_score`.
//!
//! Pure math, no I/O: callers (the code-graph indexer) build a dense
//! `0..n` node-index map and hand in `(src, dst, weight)` edges.

/// Damping factor (probability of following an edge vs teleporting).
pub const DAMPING: f64 = 0.85;
/// Iteration cap; convergence usually lands far earlier.
pub const MAX_ITERATIONS: usize = 30;
/// L1-delta convergence threshold.
pub const EPSILON: f64 = 1e-6;

/// Compute PageRank for `n` nodes over `edges` `(src, dst, weight)`.
///
/// Returns one score per node, summing to ~1.0. Deterministic: fixed
/// iteration order, `f64` throughout, no hashing. Dangling-node mass is
/// redistributed uniformly each iteration. Empty graphs (`n == 0`) return
/// an empty vec; isolated nodes receive the teleport baseline. Edges
/// referencing a node `>= n` are a caller bug: they are skipped with a
/// warning instead of panicking.
pub fn pagerank(n: usize, edges: &[(u32, u32, f64)]) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    let valid: Vec<(usize, usize, f64)> = edges
        .iter()
        .filter_map(|&(s, d, w)| {
            let (su, du) = (s as usize, d as usize);
            if su < n && du < n {
                Some((su, du, w))
            } else {
                tracing::warn!(src = s, dst = d, n, "pagerank: skipping out-of-range edge");
                None
            }
        })
        .collect();
    let mut out_weight = vec![0.0f64; n];
    for &(s, _, w) in &valid {
        out_weight[s] += w;
    }
    let teleport = (1.0 - DAMPING) / n as f64;
    let mut rank = vec![1.0 / n as f64; n];
    let mut next = vec![0.0f64; n];
    for _ in 0..MAX_ITERATIONS {
        let dangling: f64 = (0..n)
            .filter(|&i| out_weight[i] == 0.0)
            .map(|i| rank[i])
            .sum();
        let base = teleport + DAMPING * dangling / n as f64;
        next.iter_mut().for_each(|v| *v = base);
        for &(s, d, w) in &valid {
            if out_weight[s] > 0.0 {
                next[d] += DAMPING * rank[s] * w / out_weight[s];
            }
        }
        let delta: f64 = rank.iter().zip(&next).map(|(a, b)| (a - b).abs()).sum();
        std::mem::swap(&mut rank, &mut next);
        if delta < EPSILON {
            break;
        }
    }
    rank
}
