use comemory::memory::id::{is_valid_memory_id, memory_id};

#[test]
fn id_is_8_hex_prefix_of_sha256() {
    let id = memory_id("the quick brown fox");
    assert_eq!(id.len(), 8);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn id_is_stable_across_calls() {
    let a = memory_id("hello world");
    let b = memory_id("hello world");
    assert_eq!(a, b);
}

#[test]
fn id_normalizes_trailing_whitespace() {
    let a = memory_id("body text");
    let b = memory_id("body text\n\n  ");
    assert_eq!(a, b);
}

#[test]
fn generated_ids_pass_validation() {
    assert!(is_valid_memory_id(&memory_id("any body at all")));
    assert!(is_valid_memory_id("a1b2c3d4"));
    assert!(is_valid_memory_id("00000000"));
}

#[test]
fn malformed_ids_fail_validation() {
    assert!(!is_valid_memory_id("")); // empty
    assert!(!is_valid_memory_id("a1b2c3d")); // too short
    assert!(!is_valid_memory_id("a1b2c3d4e")); // too long
    assert!(!is_valid_memory_id("A1B2C3D4")); // uppercase hex
    assert!(!is_valid_memory_id("a1b2c3g4")); // non-hex char
    assert!(!is_valid_memory_id("a1b2 3d4")); // whitespace
}
