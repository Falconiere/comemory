use comemory::graph::pagerank::pagerank;

#[test]
fn pagerank_matches_hand_computed_values() {
    // 0 -> 1, 0 -> 2, 1 -> 2, 2 -> 0  (classic 3-node example, all weight 1)
    //
    // Hand derivation of the fixed point (damping d = 0.85, N = 3,
    // teleport (1-d)/3 = 0.05; node 0 splits its mass over two edges):
    //   r0 = 0.05 + 0.85*r2
    //   r1 = 0.05 + 0.85*r0/2          = 0.05 + 0.425*r0
    //   r2 = 0.05 + 0.85*(r0/2 + r1)   = 0.0925 + 0.78625*r0
    // Substituting r2 into r0:
    //   r0 = 0.05 + 0.85*(0.0925 + 0.78625*r0)
    //   r0*(1 - 0.6683125) = 0.128625
    //   r0 = 0.128625 / 0.3316875 ~= 0.38779
    //   r1 ~= 0.21481, r2 ~= 0.39740
    let scores = pagerank(3, &[(0, 1, 1.0), (0, 2, 1.0), (1, 2, 1.0), (2, 0, 1.0)]);
    assert!((scores[0] - 0.3878).abs() < 1e-3, "{scores:?}");
    assert!((scores[1] - 0.2148).abs() < 1e-3, "{scores:?}");
    assert!((scores[2] - 0.3974).abs() < 1e-3, "{scores:?}");
    assert!((scores.iter().sum::<f64>() - 1.0).abs() < 1e-9);
}

#[test]
fn pagerank_is_deterministic_and_handles_dangling() {
    let edges = [(0u32, 1u32, 2.0f64)]; // node 1 dangles, weight respected
    let a = pagerank(3, &edges);
    let b = pagerank(3, &edges);
    assert_eq!(a, b); // byte-identical across runs
    assert!((a.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    assert!(a[1] > a[2]); // 1 receives 0's mass; 2 only teleport mass
}

#[test]
fn pagerank_empty_graph_returns_empty() {
    assert!(pagerank(0, &[]).is_empty());
}

#[test]
fn pagerank_no_edges_is_uniform() {
    // All nodes dangle: their mass is redistributed uniformly each
    // iteration, so the uniform start vector is already the fixed point.
    let scores = pagerank(3, &[]);
    assert_eq!(scores, vec![1.0 / 3.0; 3]);
}

#[test]
fn pagerank_skips_out_of_range_edges_without_panicking() {
    // Edges touching nodes >= n are programmer errors from the caller's
    // index map; they must be skipped (warn), never panic.
    let scores = pagerank(2, &[(0, 7, 1.0), (9, 1, 1.0), (0, 1, 1.0)]);
    assert_eq!(scores.len(), 2);
    assert!((scores.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    // Only the in-range edge 0 -> 1 counts, so node 1 outranks node 0.
    assert!(scores[1] > scores[0], "{scores:?}");
}

#[test]
fn pagerank_weights_bias_distribution() {
    // 0 -> 1 (weight 3) vs 0 -> 2 (weight 1): node 1 gets 3x the share.
    let scores = pagerank(3, &[(0, 1, 3.0), (0, 2, 1.0)]);
    assert!(scores[1] > scores[2], "{scores:?}");
    assert!((scores.iter().sum::<f64>() - 1.0).abs() < 1e-9);
}
