//! Deterministic SplitMix64 + Beta/Gamma sampling for the bandit (no `rand`).

/// Seeded SplitMix64 PRNG. Public because it is a parameter of
/// [`crate::eval::metrics::bootstrap_ci`], whose callers own the stream so
/// that several intervals drawn in one run stay jointly reproducible.
///
/// [`SplitMix64::new`] and [`SplitMix64::next_u64`] are public for the same
/// reason: a `pub fn` taking a `pub(crate)` type trips `private_interfaces`
/// (fatal under `-D warnings`), and an external caller that owns the stream
/// must be able to construct it and draw from it. `next_f64` stays private —
/// nothing outside this module needs the unit-interval form.
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Construct from an opaque 64-bit seed.
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next raw 64-bit draw — the integer-draw entry point for `tune_sample`'s
    /// pool indices and the bootstrap's resample indices; `next_f64` covers
    /// the crate-internal unit-interval callers.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn next_f64(&mut self) -> f64 {
        let u = self.next_u64() >> 11;
        (u as f64) * (1.0 / ((1u64 << 53) as f64))
    }
}

/// Sample Beta(α, β) via two Gamma(shape, 1) draws (Marsaglia & Tsang).
/// Always returns a finite value in `[0.0, 1.0]` (falls back to `0.5`).
pub(crate) fn sample_beta(rng: &mut SplitMix64, alpha: f64, beta: f64) -> f64 {
    let x = sample_gamma(rng, alpha.max(f64::MIN_POSITIVE));
    let y = sample_gamma(rng, beta.max(f64::MIN_POSITIVE));
    let s = x + y;
    let out = if s.is_finite() && s > 0.0 { x / s } else { 0.5 };
    if out.is_finite() {
        out.clamp(0.0, 1.0)
    } else {
        0.5
    }
}

fn sample_gamma(rng: &mut SplitMix64, shape: f64) -> f64 {
    if shape < 1.0 {
        let uniform = rng.next_f64().clamp(f64::EPSILON, 1.0);
        return sample_gamma(rng, shape + 1.0) * uniform.powf(1.0 / shape);
    }
    let shape_shift = shape - 1.0 / 3.0;
    let scale = 1.0 / (9.0 * shape_shift).sqrt();
    loop {
        let mut normal;
        let mut accept;
        loop {
            normal = {
                let u1 = rng.next_f64().clamp(f64::EPSILON, 1.0 - f64::EPSILON);
                let u2 = rng.next_f64().clamp(f64::EPSILON, 1.0 - f64::EPSILON);
                (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
            };
            accept = 1.0 + scale * normal;
            if accept > 0.0 {
                break;
            }
        }
        accept = accept * accept * accept;
        let uniform = rng.next_f64().clamp(f64::EPSILON, 1.0);
        if uniform < 1.0 - 0.0331 * (normal * normal) * (normal * normal) {
            return shape_shift * accept;
        }
        if uniform.ln() < 0.5 * normal * normal + shape_shift * (1.0 - accept + accept.ln()) {
            return shape_shift * accept;
        }
    }
}

#[cfg(test)]
#[path = "tests/bandit_rng.rs"]
mod tests;
