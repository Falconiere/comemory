#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`random_hex`] draws real entropy from `/dev/urandom` — no mocked RNG.

use comemory::store::random_id::random_hex;

#[test]
fn random_hex_has_the_requested_length_and_is_lowercase_hex() {
    let hex = random_hex(8).expect("random_hex");
    assert_eq!(hex.len(), 16, "8 bytes must render as 16 hex chars");
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "must be lowercase hex: {hex}"
    );
}

#[test]
fn random_hex_draws_do_not_collide() {
    let a = random_hex(16).expect("random_hex a");
    let b = random_hex(16).expect("random_hex b");
    assert_ne!(a, b, "two independent draws from real entropy must differ");
}
