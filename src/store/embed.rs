//! Convert between f32 vectors and sqlite-vec BLOB encoding (little-
//! endian float32 buffer with no header, per sqlite-vec's vec0 wire
//! format).

use crate::prelude::*;

/// Encode an f32 vector to the BLOB layout sqlite-vec expects.
pub fn to_vec_blob(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode a BLOB into f32 vector. Errors if the BLOB length does not
/// match `expected_dim * 4`.
pub fn from_vec_blob(blob: &[u8], expected_dim: usize) -> Result<Vec<f32>> {
    if blob.len() != expected_dim * 4 {
        return Err(Error::VecDimMismatch {
            expected: expected_dim,
            got: blob.len() / 4,
        });
    }
    let mut out = Vec::with_capacity(expected_dim);
    for chunk in blob.chunks_exact(4) {
        let bytes: [u8; 4] = chunk.try_into().map_err(|_| Error::VecDimMismatch {
            expected: expected_dim,
            got: out.len(),
        })?;
        out.push(f32::from_le_bytes(bytes));
    }
    Ok(out)
}

/// Validates that `vector.len() == expected_dim`. Used at API boundaries.
pub fn guard_dim(vector: &[f32], expected_dim: usize) -> Result<()> {
    if vector.len() != expected_dim {
        return Err(Error::VecDimMismatch {
            expected: expected_dim,
            got: vector.len(),
        });
    }
    Ok(())
}
