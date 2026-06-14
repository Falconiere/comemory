use comemory::ast::{ExtractedSymbol, Lang, extract};

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
    assert_eq!(add.line_end, 1, "single-line symbol ends on its own line");
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
    // Multi-line definition: the tree-sitter span carries the true
    // inclusive end line (func opens on line 3, closes on line 5).
    assert_eq!((add.line, add.line_end), (3, 5));
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
fn rust_pub_and_async_functions_extracted() {
    // Real-world-shaped signatures lifted from this repo: most public API
    // surface carries a visibility and/or `async` modifier. Each must be
    // extracted exactly once (no double-match across the pattern table).
    let cases: &[(&str, &str)] = &[
        (
            "extract",
            "pub fn extract(lang: Lang, source: &str) -> Result<Vec<ExtractedSymbol>> {\n    run(lang, source)\n}\n",
        ),
        (
            "run_indexer",
            "pub fn run_indexer(paths: &Paths) {\n    walk(paths);\n}\n",
        ),
        (
            "dim_guard",
            "pub(crate) fn dim_guard(conn: &Connection, dim: usize) -> Result<()> {\n    check(conn, dim)\n}\n",
        ),
        ("helper", "pub(super) fn helper(x: u8) -> u8 {\n    x\n}\n"),
        (
            "fetch_embedding",
            "async fn fetch_embedding(text: &str) -> Result<Vec<f32>> {\n    call(text).await\n}\n",
        ),
        (
            "warm_cache",
            "async fn warm_cache(store: &Store) {\n    store.touch().await;\n}\n",
        ),
        (
            "sync_repo",
            "pub async fn sync_repo(repo: &str) -> Result<()> {\n    push(repo).await\n}\n",
        ),
        (
            "fire_hook",
            "pub async fn fire_hook(event: Event) {\n    dispatch(event).await;\n}\n",
        ),
    ];
    for (name, src) in cases {
        let syms = extract(Lang::Rust, src).expect("rust extraction");
        let hits: Vec<_> = syms
            .iter()
            .filter(|s| s.name == *name && s.kind == "function")
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "{name}: want exactly one match, got {syms:?}"
        );
        assert!(
            hits[0].snippet.contains("fn "),
            "{name}: snippet keeps the definition, got {:?}",
            hits[0].snippet,
        );
    }
}

#[test]
fn rust_pub_struct_enum_trait_extracted() {
    let src = "pub struct Frontmatter { pub id: String }\n\
             pub(crate) struct Paths { root: PathBuf }\n\
             pub enum Error { NotFound, VecDimMismatch }\n\
             pub trait Emitter { fn emit(&self); }\n\
             pub(crate) trait Walk { fn step(&self); }\n";
    let syms = extract(Lang::Rust, src).expect("rust extraction");
    let structs = names_of_kind(&syms, "struct");
    assert!(
        structs.contains(&"Frontmatter"),
        "pub struct in {structs:?}"
    );
    assert!(
        structs.contains(&"Paths"),
        "pub(crate) struct in {structs:?}"
    );
    assert!(names_of_kind(&syms, "enum").contains(&"Error"), "pub enum");
    let traits = names_of_kind(&syms, "trait");
    assert!(traits.contains(&"Emitter"), "pub trait in {traits:?}");
    assert!(traits.contains(&"Walk"), "pub(crate) trait in {traits:?}");
}

#[test]
fn rust_const_and_unsafe_fns_are_a_documented_gap() {
    // Pinned gap (see the `rust_patterns` comment): `const fn` and
    // `unsafe fn` items are not matched. If a pattern lands that covers
    // them, update this test together with that comment.
    for src in [
        "const fn budget() -> usize { 64 }\n",
        "pub const fn budget() -> usize { 64 }\n",
        "unsafe fn raw_read(p: *const u8) -> u8 { *p }\n",
    ] {
        let syms = extract(Lang::Rust, src).expect("rust extraction");
        assert!(syms.is_empty(), "gap changed for {src:?}: {syms:?}");
    }
}

#[test]
fn typescript_export_declarations_extracted() {
    // `export` / `export default` / `async` wrap the declaration node, and
    // extraction descends into it — no dedicated export patterns needed.
    // `abstract class` is a distinct node kind with its own pattern row.
    let src = "export function parseQuery(raw: string): Query { return lex(raw); }\n\
             export default function bootstrap() { mount(); }\n\
             export async function loadIndex(path: string): Promise<Index> { return read(path); }\n\
             export class SearchClient { query(q: string) { return run(q); } }\n\
             export default class App { render() { return null; } }\n\
             export abstract class BaseStore { abstract flush(): void; }\n";
    let syms = extract(Lang::Typescript, src).expect("ts extraction");
    let fns = names_of_kind(&syms, "function");
    for name in ["parseQuery", "bootstrap", "loadIndex"] {
        assert!(fns.contains(&name), "missing function {name} in {fns:?}");
    }
    let classes = names_of_kind(&syms, "class");
    for name in ["SearchClient", "App", "BaseStore"] {
        assert!(
            classes.contains(&name),
            "missing class {name} in {classes:?}"
        );
    }
}

#[test]
fn python_decorated_async_and_based_definitions_extracted() {
    // Decorators and `async` wrap the definition node and extraction
    // descends into it; a base-class list needs its own pattern row.
    let src = "@app.route(\"/health\")\ndef health():\n    return ok()\n\n\
             @staticmethod\n@cached\ndef stacked():\n    return 1\n\n\
             async def fetch_rows(conn):\n    return await conn.fetch()\n\n\
             @dataclass\nclass Record:\n    pass\n\n\
             class Indexer(BaseIndexer):\n    def run(self):\n        pass\n";
    let syms = extract(Lang::Python, src).expect("python extraction");
    let fns = names_of_kind(&syms, "function");
    for name in ["health", "stacked", "fetch_rows"] {
        assert!(fns.contains(&name), "missing function {name} in {fns:?}");
    }
    let classes = names_of_kind(&syms, "class");
    for name in ["Record", "Indexer"] {
        assert!(
            classes.contains(&name),
            "missing class {name} in {classes:?}"
        );
    }
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
