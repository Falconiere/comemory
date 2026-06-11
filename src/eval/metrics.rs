//! Pure retrieval-quality metrics: recall@k and MRR building blocks.

/// Fraction of `relevant` ids appearing in the first `k` of `returned`.
/// An empty `relevant` set scores 0.0 — a golden pair with no live
/// relevant ids carries no signal and must not inflate the average.
pub fn recall_at_k(relevant: &[String], returned: &[String], k: usize) -> f64 {
    if relevant.is_empty() {
        return 0.0;
    }
    let top: std::collections::HashSet<&str> =
        returned.iter().take(k).map(String::as_str).collect();
    let hit = relevant.iter().filter(|r| top.contains(r.as_str())).count();
    hit as f64 / relevant.len() as f64
}

/// One-based rank of the first relevant id in `returned`, or `None`.
/// `1 / rank` summed over queries / query count = MRR.
pub fn first_hit_rank(relevant: &[String], returned: &[String]) -> Option<usize> {
    let rel: std::collections::HashSet<&str> = relevant.iter().map(String::as_str).collect();
    returned
        .iter()
        .position(|r| rel.contains(r.as_str()))
        .map(|p| p + 1)
}
