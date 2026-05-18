//! Scoring helpers used by the retrieval pipeline. Kept allocation-light and
//! pure so they can be reused by later fusion / reranking layers.

/// Z-score normalize `xs` in place-returning form: each output element is
/// `(x - mean) / sd`. Empty input returns an empty vec. A tiny epsilon
/// (`1e-9`) protects against divide-by-zero when all inputs are equal.
pub fn z_normalize(xs: &[f32]) -> Vec<f32> {
    if xs.is_empty() {
        return Vec::new();
    }
    let n = xs.len() as f32;
    let mean: f32 = xs.iter().sum::<f32>() / n;
    let var: f32 = xs.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / n;
    let sd = var.sqrt().max(1e-9);
    xs.iter().map(|x| (x - mean) / sd).collect()
}

/// Confidence gap: `top1 - top2` on an already-descending slice. Returns the
/// only value when length 1, and `0.0` for an empty slice — both treated as
/// "no second result to compare against".
pub fn confidence_gap(sorted_desc: &[f32]) -> f32 {
    match sorted_desc {
        [] => 0.0,
        [a] => *a,
        [a, b, ..] => a - b,
    }
}
