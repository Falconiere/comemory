//! Test mirror for `src/store/embed.rs`.
//!
//! Covers the `to_vec_blob` / `from_vec_blob` round-trip and the dim
//! guard that protects every API boundary.

#[path = "common/vectors.rs"]
mod vectors;

use comemory::store::embed;

#[test]
fn vec_blob_roundtrip_preserves_floats() {
    let original = vectors::vector("roundtrip", 1024);
    let blob = embed::to_vec_blob(&original);
    let decoded = embed::from_vec_blob(&blob, 1024).expect("decode");
    assert_eq!(decoded.len(), 1024);
    for (a, b) in original.iter().zip(&decoded) {
        assert!((a - b).abs() < f32::EPSILON, "{a} != {b}");
    }
}

#[test]
fn dim_guard_rejects_wrong_dim() {
    let blob = embed::to_vec_blob(&vectors::vector("short", 4));
    let err = embed::from_vec_blob(&blob, 8).expect_err("dim mismatch");
    let msg = format!("{err}");
    assert!(msg.contains("dim mismatch"), "got: {msg}");
}
