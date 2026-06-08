use comemory::simhash;

#[test]
fn similar_strings_have_close_simhash() {
    let a = simhash::simhash64(tokenize("the quick brown fox jumps over the lazy dog"));
    let b = simhash::simhash64(tokenize("the quick brown fox leaps over the lazy dog"));
    let d = simhash::hamming64(a, b);
    assert!(d < 12, "expected close, got hamming={d}");
}

#[test]
fn unrelated_strings_have_far_simhash() {
    let a = simhash::simhash64(tokenize("authentication middleware for express"));
    let b = simhash::simhash64(tokenize("postgres advisory lock migration ordering"));
    let d = simhash::hamming64(a, b);
    assert!(d > 20, "expected far, got hamming={d}");
}

fn tokenize(s: &str) -> impl Iterator<Item = &str> {
    s.split_whitespace()
}
