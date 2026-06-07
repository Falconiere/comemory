use comemory::ast::Lang;

#[test]
fn rust_extension_resolves() {
    assert_eq!(Lang::from_extension("rs"), Some(Lang::Rust));
}

#[test]
fn typescript_extensions_resolve() {
    assert_eq!(Lang::from_extension("ts"), Some(Lang::TypeScript));
    // `.tsx` now resolves to the dedicated `Tsx` variant so JSX-bearing
    // source uses the right tree-sitter grammar.
    assert_eq!(Lang::from_extension("tsx"), Some(Lang::Tsx));
}

#[test]
fn javascript_extensions_resolve() {
    assert_eq!(Lang::from_extension("js"), Some(Lang::JavaScript));
    assert_eq!(Lang::from_extension("jsx"), Some(Lang::JavaScript));
    assert_eq!(Lang::from_extension("mjs"), Some(Lang::JavaScript));
    assert_eq!(Lang::from_extension("cjs"), Some(Lang::JavaScript));
}

#[test]
fn python_extension_resolves() {
    assert_eq!(Lang::from_extension("py"), Some(Lang::Python));
}

#[test]
fn unknown_extension_is_none() {
    assert_eq!(Lang::from_extension("md"), None);
    assert_eq!(Lang::from_extension(""), None);
    assert_eq!(
        Lang::from_extension("RS"),
        None,
        "case-sensitive on purpose"
    );
}

#[test]
fn as_str_returns_canonical_name() {
    assert_eq!(Lang::Rust.as_str(), "rust");
    assert_eq!(Lang::TypeScript.as_str(), "typescript");
    assert_eq!(Lang::Tsx.as_str(), "tsx");
    assert_eq!(Lang::JavaScript.as_str(), "javascript");
    assert_eq!(Lang::Python.as_str(), "python");
}
