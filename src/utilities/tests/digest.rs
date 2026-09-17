//! Tests for the shared digest/hex primitives extracted from `memory::id`
//! so `memory::id` and `utilities::query_id` cannot drift on the hex shape.

use comemory::utilities::digest::{is_lower_hex, sha256_hex};

#[test]
fn sha256_hex_matches_the_known_empty_digest() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn sha256_hex_is_lowercase_and_64_chars() {
    let hex = sha256_hex(b"Cafe\xcc\x81 notes");
    assert_eq!(hex.len(), 64);
    assert!(
        is_lower_hex(&hex, 64),
        "digest must be lowercase hex: {hex}"
    );
}

#[test]
fn is_lower_hex_accepts_exactly_the_expected_length() {
    assert!(is_lower_hex("0123abcd", 8));
    assert!(!is_lower_hex("0123abc", 8), "seven chars is not eight");
    assert!(!is_lower_hex("0123abcde", 8), "nine chars is not eight");
}

#[test]
fn is_lower_hex_rejects_uppercase_and_non_hex() {
    assert!(!is_lower_hex("0123ABCD", 8), "uppercase is rejected");
    assert!(!is_lower_hex("0123abcg", 8), "`g` is not hex");
    assert!(!is_lower_hex("0123 bcd", 8), "a space is not hex");
}
