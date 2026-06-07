use comemory::index::MemoryHit;
use comemory::memory::Kind;
use comemory::retrieval::corrective::should_fallback;

fn hit(score: f32) -> MemoryHit {
    MemoryHit {
        id: "x".into(),
        score,
        body: String::new(),
        kind: Kind::Note,
        repo: "r".into(),
    }
}

#[test]
fn fallback_when_gap_below_min() {
    let hits = vec![hit(0.9), hit(0.89), hit(0.88)];
    assert!(should_fallback(&hits, 0.15));
}

#[test]
fn no_fallback_when_gap_above_min() {
    let hits = vec![hit(0.9), hit(0.6), hit(0.4)];
    assert!(!should_fallback(&hits, 0.15));
}

#[test]
fn fallback_when_fewer_than_three_hits() {
    // Even with a huge gap, sparse results trigger the fallback so the
    // pipeline gets a chance to broaden.
    let hits = vec![hit(0.99), hit(0.10)];
    assert!(should_fallback(&hits, 0.05));
    assert!(should_fallback(&[hit(0.99)], 0.05));
    assert!(should_fallback(&[], 0.05));
}

#[test]
fn fallback_boundary_strictly_less_than() {
    // Gap exactly equals min_confidence => no fallback (strict `<`).
    // Use exactly-representable f32 binary fractions so the equality
    // path actually fires (e.g. 0.5 and 0.25 round-trip without loss).
    let hits = vec![hit(1.0), hit(0.5), hit(0.25)];
    assert!(!should_fallback(&hits, 0.5));
}
