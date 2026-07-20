//! Mirror for `src/eval/bandit_rng.rs`. The SplitMix64 / `sample_beta`
//! helpers are `pub(crate)`, so this binary pins them through the public
//! `bandit::thompson_sample` / `bandit::sample_seed` surface they feed.

use comemory::eval::bandit::{self, Arm};
use comemory::eval::tune::TuneCandidate;

#[test]
fn sample_seed_is_stable_for_same_inputs() {
    let a = bandit::sample_seed(10, 81);
    let b = bandit::sample_seed(10, 81);
    assert_eq!(a, b, "sample_seed must be a pure function of its inputs");
    assert_ne!(
        bandit::sample_seed(10, 81),
        bandit::sample_seed(11, 81),
        "golden-pair count must mix into the seed"
    );
    assert_ne!(
        bandit::sample_seed(10, 81),
        bandit::sample_seed(10, 80),
        "arm count must mix into the seed"
    );
}

#[test]
fn thompson_sample_exercises_rng_deterministically() {
    // Two equal Beta(1,1) arms: the sampler walks bandit_rng::sample_beta.
    // Same seed → same winner across calls (documents rng stability).
    let arms = [
        Arm {
            arm_id: "left".into(),
            candidate: TuneCandidate {
                rrf_k: 20.0,
                decay: 0.3,
                mmr_lambda: 0.5,
                bm25_weights: (1.0, 1.0),
            },
            alpha: 1.0,
            beta: 1.0,
            pulls: 0,
            last_mrr: None,
        },
        Arm {
            arm_id: "right".into(),
            candidate: TuneCandidate {
                rrf_k: 100.0,
                decay: 0.8,
                mmr_lambda: 0.9,
                bm25_weights: (2.0, 1.0),
            },
            alpha: 1.0,
            beta: 1.0,
            pulls: 0,
            last_mrr: None,
        },
    ];
    let seed = bandit::sample_seed(7, arms.len());
    let w1 = bandit::thompson_sample(&arms, seed).expect("first");
    let w2 = bandit::thompson_sample(&arms, seed).expect("second");
    assert_eq!(w1.arm_id, w2.arm_id);
}
