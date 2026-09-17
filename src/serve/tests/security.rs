#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The HTTP security core: token comparison, the loopback Host guard, and
//! token generation. The canonicalize-and-contain path check moved to
//! `utilities::path_containment` with #166 and is asserted there.

use comemory::serve::security::{generate_token, host_is_loopback, token_matches};

#[test]
fn token_matches_only_on_exact() {
    assert!(token_matches(Some("abc"), "abc"));
    assert!(!token_matches(Some("abc"), "abcd"));
    assert!(!token_matches(Some(""), "abc"));
    assert!(!token_matches(None, "abc"));
}

#[test]
fn host_loopback_accepts_only_loopback() {
    for ok in [
        "127.0.0.1",
        "127.0.0.1:8799",
        "localhost",
        "localhost:3000",
        "::1",
        "[::1]:8799",
    ] {
        assert!(host_is_loopback(ok), "{ok} should be loopback");
    }
    for bad in [
        "",
        "evil.com",
        "evil.com:80",
        "127.0.0.1.evil.com",
        "10.0.0.1",
    ] {
        assert!(!host_is_loopback(bad), "{bad} should be rejected");
    }
}

#[test]
fn generate_token_is_64_hex_and_unique() {
    let a = generate_token().expect("token a");
    let b = generate_token().expect("token b");
    assert_eq!(a.len(), 64, "256 bits → 64 hex chars");
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b, "tokens must not repeat");
}
