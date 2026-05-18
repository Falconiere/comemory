use qwick::memory::id::memory_id;

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
