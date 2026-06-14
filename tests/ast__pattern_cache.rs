//! Tests for the compile-once pattern cache behind `comemory`'s symbol and
//! import extraction.
//!
//! The cache's `cached` accessor is `pub(crate)`, so these tests exercise it
//! through the public `extract` / `extract_imports` surface — which is also
//! the byte-identical guard: a broken cache (wrong grammar reused, dropped
//! pattern, reordered table) would change the extracted symbol list.

use comemory::ast::{Lang, extract};
use comemory::graph::imports::extract_imports;

/// Extraction must be byte-identical across repeated calls: the first call
/// compiles the table into the process `static`, every later call reuses it.
/// Equal output across calls proves the cached patterns are applied
/// faithfully (same set, same order, same fields).
#[test]
fn extraction_is_identical_across_repeated_calls() {
    let src = "pub fn alpha(x: i32) -> i32 { x }\n\
               struct Beta { y: u8 }\n\
               pub async fn gamma() { run().await; }\n\
               enum Delta { A, B }\n\
               pub trait Epsilon { fn step(&self); }\n";
    let first = extract(Lang::Rust, src).expect("first extraction");
    // Repeat enough times that the cache is unambiguously warm.
    for _ in 0..5 {
        let again = extract(Lang::Rust, src).expect("repeat extraction");
        assert_eq!(again, first, "cached extraction diverged from first run");
    }
    // Sanity: the run actually extracted the symbols (not an empty match).
    let names: Vec<&str> = first.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "gamma", "Beta", "Delta", "Epsilon"]);
}

/// Interleaving languages must not let one language's cached table leak into
/// another (each language owns a distinct cell). Extracting Rust, then TS,
/// then Rust again must yield the same Rust result as a Rust-only run.
#[test]
fn interleaved_languages_keep_separate_cached_tables() {
    let rust = "fn r() -> u8 { 0 }\n";
    let ts = "function t(a: number): number { return a; }\n";
    let rust_baseline = extract(Lang::Rust, rust).expect("rust baseline");

    let _ = extract(Lang::Typescript, ts).expect("ts extraction");
    let rust_after_ts = extract(Lang::Rust, rust).expect("rust after ts");
    assert_eq!(
        rust_after_ts, rust_baseline,
        "Rust extraction changed after a TypeScript extraction ran",
    );
}

/// The shared TypeScript/JavaScript import patterns compile under two
/// grammars; each grammar must own its own cached table. Tsx accepts JSX,
/// JavaScript does not — extracting from one must not corrupt the other's
/// cached patterns. Both must still recover their imports across calls.
#[test]
fn ts_and_js_import_caches_do_not_cross_contaminate() {
    let ts = "import { a } from './a';\nimport React from \"react\";\n";
    let js = "const fs = require('fs');\nconst legacy = require(\"./legacy\");\n";

    let ts_first = extract_imports(Lang::Typescript, ts).expect("ts imports");
    let js_first = extract_imports(Lang::Javascript, js).expect("js imports");

    // Re-run interleaved; cached tables must be stable and grammar-correct.
    let ts_again = extract_imports(Lang::Typescript, ts).expect("ts imports 2");
    let js_again = extract_imports(Lang::Javascript, js).expect("js imports 2");

    assert_eq!(ts_again, ts_first, "TS import extraction diverged");
    assert_eq!(js_again, js_first, "JS import extraction diverged");
    assert!(ts_first.contains(&"./a".to_string()), "{ts_first:?}");
    assert!(ts_first.contains(&"react".to_string()), "{ts_first:?}");
    assert!(js_first.contains(&"fs".to_string()), "{js_first:?}");
    assert!(js_first.contains(&"./legacy".to_string()), "{js_first:?}");
}

/// Each supported language extracts independently after the cache is warm
/// for all of them — a multi-file repo walk hits every cell, in any order,
/// and every language keeps producing its own symbols.
#[test]
fn every_language_extracts_after_all_caches_warm() {
    let cases: &[(Lang, &str, &str)] = &[
        (Lang::Rust, "fn rs() {}\n", "rs"),
        (Lang::Typescript, "function ts() {}\n", "ts"),
        (Lang::Javascript, "function js() {}\n", "js"),
        (Lang::Python, "def py():\n    pass\n", "py"),
        (Lang::Go, "package m\nfunc Go() {}\n", "Go"),
    ];
    // Warm every cache first.
    for (lang, src, _) in cases {
        let _ = extract(*lang, src).expect("warm");
    }
    // Now re-extract each and confirm the expected symbol is present.
    for (lang, src, name) in cases {
        let syms = extract(*lang, src).expect("warm extraction");
        assert!(
            syms.iter().any(|s| s.name == *name),
            "{lang:?}: missing {name} in {syms:?}",
        );
    }
}
