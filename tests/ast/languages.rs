use comemory::ast::languages::{detect, supported};
use comemory::ast::Lang;
use std::path::Path;

#[test]
fn rust_extension_resolves() {
    assert_eq!(detect(Path::new("foo.rs")), Some(Lang::Rust));
}

#[test]
fn typescript_extensions_resolve() {
    assert_eq!(detect(Path::new("foo.ts")), Some(Lang::Typescript));
    // `.tsx` now folds into the single `Typescript` variant — the
    // extractor + pattern dispatch use the Tsx grammar internally to keep
    // JSX-bearing files parsing.
    assert_eq!(detect(Path::new("foo.tsx")), Some(Lang::Typescript));
}

#[test]
fn javascript_extensions_resolve() {
    assert_eq!(detect(Path::new("foo.js")), Some(Lang::Javascript));
    assert_eq!(detect(Path::new("foo.jsx")), Some(Lang::Javascript));
    assert_eq!(detect(Path::new("foo.mjs")), Some(Lang::Javascript));
    assert_eq!(detect(Path::new("foo.cjs")), Some(Lang::Javascript));
}

#[test]
fn python_extension_resolves() {
    assert_eq!(detect(Path::new("foo.py")), Some(Lang::Python));
}

#[test]
fn go_extension_resolves() {
    assert_eq!(detect(Path::new("foo.go")), Some(Lang::Go));
}

#[test]
fn unknown_extension_is_none() {
    assert_eq!(detect(Path::new("foo.md")), None);
    assert_eq!(detect(Path::new("foo")), None);
}

#[test]
fn detect_is_case_insensitive() {
    // The detector lower-cases the extension before matching so common
    // capitalised forms (`FOO.RS`, `Foo.PY`) still resolve.
    assert_eq!(detect(Path::new("FOO.RS")), Some(Lang::Rust));
    assert_eq!(detect(Path::new("Foo.PY")), Some(Lang::Python));
}

#[test]
fn as_str_returns_canonical_name() {
    assert_eq!(Lang::Rust.as_str(), "rust");
    assert_eq!(Lang::Typescript.as_str(), "typescript");
    assert_eq!(Lang::Javascript.as_str(), "javascript");
    assert_eq!(Lang::Python.as_str(), "python");
    assert_eq!(Lang::Go.as_str(), "go");
}

#[test]
fn parse_accepts_canonical_and_short_aliases() {
    assert_eq!(Lang::parse("rust"), Some(Lang::Rust));
    assert_eq!(Lang::parse("rs"), Some(Lang::Rust));
    assert_eq!(Lang::parse("typescript"), Some(Lang::Typescript));
    assert_eq!(Lang::parse("ts"), Some(Lang::Typescript));
    assert_eq!(Lang::parse("tsx"), Some(Lang::Typescript));
    assert_eq!(Lang::parse("javascript"), Some(Lang::Javascript));
    assert_eq!(Lang::parse("js"), Some(Lang::Javascript));
    assert_eq!(Lang::parse("jsx"), Some(Lang::Javascript));
    assert_eq!(Lang::parse("python"), Some(Lang::Python));
    assert_eq!(Lang::parse("py"), Some(Lang::Python));
    assert_eq!(Lang::parse("go"), Some(Lang::Go));
}

#[test]
fn parse_rejects_unsupported() {
    assert_eq!(Lang::parse("ruby"), None);
    assert_eq!(Lang::parse(""), None);
    assert_eq!(Lang::parse("RS"), None, "case-sensitive on purpose");
}

#[test]
fn supported_returns_five_canonical_names() {
    assert_eq!(
        supported(),
        &["rust", "typescript", "javascript", "python", "go"]
    );
}
