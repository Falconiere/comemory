use comemory::ast::{extract, ExtractedSymbol, Lang};

fn names_of_kind<'a>(syms: &'a [ExtractedSymbol], kind: &str) -> Vec<&'a str> {
    syms.iter()
        .filter(|s| s.kind == kind)
        .map(|s| s.name.as_str())
        .collect()
}

#[test]
fn rust_functions_extracted_with_lines() {
    let src = "fn add(a: i32, b: i32) -> i32 { a + b }\n\
             fn sub(a: i32, b: i32) -> i32 { a - b }\n";
    let syms = extract(Lang::Rust, src).expect("rust extraction");
    let fns = names_of_kind(&syms, "function");
    assert!(fns.contains(&"add"), "missing add in {fns:?}");
    assert!(fns.contains(&"sub"), "missing sub in {fns:?}");
    // lines are one-based and stable across the two definitions.
    let add = syms.iter().find(|s| s.name == "add").expect("add sym");
    assert_eq!(add.line, 1);
    assert_eq!(add.language, "rust");
    assert!(add.snippet.contains("fn add"));
    // Symbols under the chunk line budget are stored whole.
    assert!(add.chunks.is_empty(), "small symbol must stay unchunked");
    let sub = syms.iter().find(|s| s.name == "sub").expect("sub sym");
    assert_eq!(sub.line, 2);
}

#[test]
fn rust_struct_enum_trait_extracted() {
    let src = "struct Foo { x: i32 }\n\
             enum Bar { A, B }\n\
             trait Quux { fn q(&self); }\n";
    let syms = extract(Lang::Rust, src).expect("rust extraction");
    assert!(names_of_kind(&syms, "struct").contains(&"Foo"));
    assert!(names_of_kind(&syms, "enum").contains(&"Bar"));
    assert!(names_of_kind(&syms, "trait").contains(&"Quux"));
}

#[test]
fn python_function_and_class_extracted() {
    let src = "class Foo:\n    def bar(self):\n        pass\n\ndef top():\n    return 1\n";
    let syms = extract(Lang::Python, src).expect("python extraction");
    assert!(
        syms.iter()
            .any(|s| s.name == "Foo" && s.kind == "class" && s.language == "python"),
        "missing class Foo in {syms:?}",
    );
    assert!(
        syms.iter().any(|s| s.name == "top" && s.kind == "function"),
        "missing function top in {syms:?}",
    );
}

#[test]
fn typescript_function_and_class_extracted() {
    let src = "function add(a: number, b: number): number { return a + b; }\n\
             class Greeter { hello(name: string) { return `hi ${name}`; } }\n";
    let syms = extract(Lang::Typescript, src).expect("ts extraction");
    assert!(
        syms.iter().any(|s| s.name == "add" && s.kind == "function"),
        "missing function add in {syms:?}",
    );
    assert!(
        syms.iter()
            .any(|s| s.name == "Greeter" && s.kind == "class"),
        "missing class Greeter in {syms:?}",
    );
    // language tag must match the requested lang.
    assert!(syms.iter().all(|s| s.language == "typescript"));
}

#[test]
fn javascript_function_extracted() {
    let src = "function add(a, b) { return a + b; }\n";
    let syms = extract(Lang::Javascript, src).expect("js extraction");
    let add = syms
        .iter()
        .find(|s| s.name == "add" && s.kind == "function")
        .expect("missing js function add");
    assert_eq!(add.language, "javascript");
    assert!(add.snippet.contains("function add"));
}

#[test]
fn go_function_extracted() {
    let src = "package main\n\nfunc add(a int, b int) int {\n\treturn a + b\n}\n";
    let syms = extract(Lang::Go, src).expect("go extraction");
    let add = syms
        .iter()
        .find(|s| s.name == "add" && s.kind == "function")
        .expect("missing go function add");
    assert_eq!(add.language, "go");
    assert!(add.snippet.contains("func add"));
    assert!(add.chunks.is_empty(), "small symbol must stay unchunked");
}

#[test]
fn empty_source_yields_no_symbols() {
    for lang in [
        Lang::Rust,
        Lang::Typescript,
        Lang::Javascript,
        Lang::Python,
        Lang::Go,
    ] {
        let syms = extract(lang, "").expect("extract empty");
        assert!(syms.is_empty(), "{lang:?} produced symbols from empty src");
    }
}

#[test]
fn non_matching_source_yields_no_symbols() {
    // Plain expression — not a definition.
    let syms = extract(Lang::Rust, "let _x = 1;").expect("extract");
    assert!(syms.is_empty(), "expected no syms, got {syms:?}");
}

#[test]
fn tsx_jsx_component_extracted() {
    // A function returning JSX parses cleanly under the Tsx grammar (which
    // `Lang::Typescript` now dispatches to internally), so callers don't
    // need a separate variant for JSX-bearing source.
    let src = "function Hello() { return <div />; }\n";
    let syms = extract(Lang::Typescript, src).expect("tsx extraction");
    let hello = syms
        .iter()
        .find(|s| s.name == "Hello" && s.kind == "function")
        .expect("missing function Hello in JSX-bearing source");
    assert_eq!(hello.language, "typescript", "language tag is canonical");
    assert!(
        hello.snippet.contains("<div"),
        "snippet should retain JSX element, got {:?}",
        hello.snippet,
    );
}
