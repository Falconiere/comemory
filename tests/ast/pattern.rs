use comemory::ast::pattern::find;
use comemory::ast::Lang;

#[test]
fn pattern_matches_function_call_in_rust() {
    let src = "fn main() {\n    foo(1, 2);\n    bar(3);\n}\n";
    let hits = find(Lang::Rust, src, "$F($$$ARGS)").expect("rust pattern");
    assert!(!hits.is_empty(), "expected at least one call match");
    // We should pick up `foo(1, 2)` on line 2 and `bar(3)` on line 3.
    let snippets: Vec<&str> = hits.iter().map(|(_, s)| s.as_str()).collect();
    assert!(
        snippets.iter().any(|s| s.starts_with("foo(")),
        "missing foo() in {snippets:?}",
    );
    assert!(
        snippets.iter().any(|s| s.starts_with("bar(")),
        "missing bar() in {snippets:?}",
    );
}

#[test]
fn pattern_returns_one_based_line_numbers() {
    let src = "fn a() {}\n\nfn b() { println!(\"hi\"); }\n";
    let hits = find(Lang::Rust, src, "println!($$$ARGS)").expect("rust pattern");
    let lines: Vec<usize> = hits.iter().map(|(l, _)| *l).collect();
    assert_eq!(
        lines,
        vec![3],
        "expected single hit on line 3, got {lines:?}"
    );
}

#[test]
fn pattern_matches_python_function_call() {
    let src = "def main():\n    foo(1, 2)\n    bar()\n";
    let hits = find(Lang::Python, src, "$F($$$ARGS)").expect("python pattern");
    assert!(
        hits.iter().any(|(_, s)| s.starts_with("foo(")),
        "missing foo() in {hits:?}",
    );
    assert!(
        hits.iter().any(|(_, s)| s.starts_with("bar(")),
        "missing bar() in {hits:?}",
    );
}

#[test]
fn pattern_matches_ts_function_call() {
    let src = "function main() { foo(1, 2); bar(); }\n";
    let hits = find(Lang::TypeScript, src, "$F($$$ARGS)").expect("ts pattern");
    assert!(
        hits.iter().any(|(_, s)| s.starts_with("foo(")),
        "missing foo() in {hits:?}",
    );
}

#[test]
fn invalid_pattern_returns_error() {
    // Empty pattern is rejected by ast-grep — we surface that as an Err
    // instead of panicking.
    let err = find(Lang::Rust, "fn main() {}", "").expect_err("expected err");
    let msg = format!("{err}");
    assert!(msg.contains("ast-grep pattern"), "unexpected msg: {msg}");
}

#[test]
fn pattern_with_no_matches_returns_empty() {
    let src = "fn main() {}\n";
    let hits = find(Lang::Rust, src, "println!($$$ARGS)").expect("rust pattern");
    assert!(hits.is_empty(), "expected no hits, got {hits:?}");
}
