//! Deterministic float32 vectors for benches. Mirrors
//! `tests/common/vectors.rs::vector` (benches are a separate compilation
//! unit and cannot import `tests/common/`). The seed drives the bytes so
//! repeated runs synthesize identical corpora — perf numbers stay
//! comparable across runs.

use sha2::{Digest, Sha256};

/// Produce a `dim`-dimensional vector deterministically from `seed`.
///
/// `state = Sha256(seed)`; each 4-byte LE chunk maps `u32 -> [-1, 1]`; when
/// the 32-byte block is exhausted and more components are needed, the state
/// is re-hashed (`state = Sha256(state)`). No L2 normalization — the vec0
/// KNN uses cosine distance and the raw spread exercises it fine.
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
            let bytes: [u8; 4] = chunk.try_into().unwrap();
            let v = u32::from_le_bytes(bytes) as f32 / u32::MAX as f32;
            out.push(v * 2.0 - 1.0); // map to [-1, 1]
        }
        let mut next = Sha256::new();
        next.update(state);
        state = next.finalize().into();
    }
    out
}
