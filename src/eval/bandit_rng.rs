//! Deterministic SplitMix64 + Beta/Gamma sampling for the bandit (no `rand`).

/// Seeded SplitMix64 PRNG.
pub(crate) struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Construct from an opaque 64-bit seed.
    pub(crate) fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    fn next_f64(&mut self) -> f64 {
        let u = self.next_u64() >> 11;
        (u as f64) * (1.0 / ((1u64 << 53) as f64))
    }
}

/// Sample Beta(α, β) via two Gamma(shape, 1) draws (Marsaglia & Tsang).
pub(crate) fn sample_beta(rng: &mut SplitMix64, alpha: f64, beta: f64) -> f64 {
    let x = sample_gamma(rng, alpha);
    let y = sample_gamma(rng, beta);
    let s = x + y;
    if s <= 0.0 { 0.5 } else { x / s }
}

fn sample_gamma(rng: &mut SplitMix64, shape: f64) -> f64 {
    if shape < 1.0 {
        let u = rng.next_f64().clamp(f64::EPSILON, 1.0);
        return sample_gamma(rng, shape + 1.0) * u.powf(1.0 / shape);
    }
    let d = shape - 1.0 / 3.0;
    let c = 1.0 / (9.0 * d).sqrt();
    loop {
        let mut x;
        let mut v;
        loop {
            x = {
                let u1 = rng.next_f64().clamp(f64::EPSILON, 1.0 - f64::EPSILON);
                let u2 = rng.next_f64().clamp(f64::EPSILON, 1.0 - f64::EPSILON);
                (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
            };
            v = 1.0 + c * x;
            if v > 0.0 {
                break;
            }
        }
        v = v * v * v;
        let u = rng.next_f64().clamp(f64::EPSILON, 1.0);
        if u < 1.0 - 0.0331 * (x * x) * (x * x) {
            return d * v;
        }
        if u.ln() < 0.5 * x * x + d * (1.0 - v + v.ln()) {
            return d * v;
        }
    }
}
