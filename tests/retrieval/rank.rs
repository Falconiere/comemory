use comemory::retrieval::rank::{confidence_gap, rrf_fuse, z_normalize};

#[test]
fn z_normalize_centers_around_zero() {
    let xs = vec![1.0_f32, 2.0, 3.0, 4.0];
    let z = z_normalize(&xs);
    let sum: f32 = z.iter().sum();
    assert!(sum.abs() < 1e-5, "expected zero-mean output, got sum {sum}");
}

#[test]
fn z_normalize_unit_variance() {
    let xs = vec![1.0_f32, 2.0, 3.0, 4.0];
    let z = z_normalize(&xs);
    let n = z.len() as f32;
    let var: f32 = z.iter().map(|x| x * x).sum::<f32>() / n;
    assert!(
        (var - 1.0).abs() < 1e-4,
        "expected unit variance, got {var}"
    );
}

#[test]
fn z_normalize_empty_input_returns_empty() {
    assert!(z_normalize(&[]).is_empty());
}

#[test]
fn z_normalize_constant_input_does_not_divide_by_zero() {
    let xs = vec![5.0_f32; 4];
    let z = z_normalize(&xs);
    // With epsilon-floored sd, every element collapses to 0.0 cleanly.
    assert_eq!(z.len(), 4);
    for v in &z {
        assert!(v.abs() < 1e-3, "expected ~0 for constant input, got {v}");
    }
}

#[test]
fn confidence_gap_is_top1_minus_top2() {
    assert!((confidence_gap(&[0.9, 0.7, 0.5]) - 0.2_f32).abs() < 1e-6);
    assert_eq!(confidence_gap(&[0.5]), 0.5);
    assert_eq!(confidence_gap(&[]), 0.0);
}

#[test]
fn rrf_fuse_empty_inputs_returns_empty() {
    let out: Vec<(String, f32)> = rrf_fuse::<&str>(&[], 60.0);
    assert!(out.is_empty());
}

#[test]
fn rrf_fuse_single_ranking_preserves_order() {
    let ranking = vec!["a", "b", "c"];
    let out = rrf_fuse::<&str>(&[&ranking], 60.0);
    let ids: Vec<&str> = out.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b", "c"]);
}

#[test]
fn rrf_fuse_two_rankings_boosts_intersection() {
    let r1 = vec!["a", "b", "c"];
    let r2 = vec!["c", "a", "d"];
    let out = rrf_fuse::<&str>(&[&r1, &r2], 60.0);
    let ids: Vec<&str> = out.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, vec!["a", "c", "b", "d"]);
}

#[test]
fn rrf_fuse_stable_tie_break_by_id() {
    let r1 = vec!["b", "a"];
    let r2 = vec!["a", "b"];
    let out = rrf_fuse::<&str>(&[&r1, &r2], 60.0);
    let ids: Vec<&str> = out.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b"]);
}
