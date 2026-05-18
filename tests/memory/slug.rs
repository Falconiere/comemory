use qwick::memory::slug::slug_from_body;

#[test]
fn slug_from_first_meaningful_line() {
    let s = slug_from_body("decision: use Postgres for analytics");
    assert_eq!(s, "decision-use-postgres-for-analytics");
}

#[test]
fn slug_truncates_to_max_chars() {
    let body = "a".repeat(200);
    assert_eq!(slug_from_body(&body).len(), 60);
}

#[test]
fn slug_falls_back_when_only_whitespace() {
    assert_eq!(slug_from_body("\n\n  "), "untitled");
}

#[test]
fn slug_only_keeps_ascii_alphanumeric_and_dashes() {
    let s = slug_from_body("Café — über 100%!");
    for c in s.chars() {
        assert!(c.is_ascii_alphanumeric() || c == '-', "bad char: {c}");
    }
}
