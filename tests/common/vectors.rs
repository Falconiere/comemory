//! Deterministic float32 vectors for tests. Never call random; the
//! seed is derived from the input so order-asserting tests stay
//! stable across CI runs.

use sha2::{Digest, Sha256};

/// Produce a `dim`-dimensional vector deterministically from `seed`.
pub fn vector(seed: &str, dim: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(dim);
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    let mut state: [u8; 32] = hasher.finalize().into();
    while out.len() < dim {
        for chunk in state.chunks(4) {
            if out.len() == dim {
                break;
            }
            let bytes: [u8; 4] = chunk.try_into().expect("chunks(4) yields [u8;4]");
            let v = u32::from_le_bytes(bytes) as f32 / u32::MAX as f32;
            out.push(v * 2.0 - 1.0); // map to [-1, 1]
        }
        let mut next = Sha256::new();
        next.update(state);
        state = next.finalize().into();
    }
    out
}
