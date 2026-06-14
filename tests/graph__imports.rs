use comemory::ast::languages::Lang;
use comemory::graph::imports::{PathIndex, extract_imports};

fn paths(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn rust_use_and_mod_yield_module_names() {
    let src = "use crate::store::fts;\nmod tokenizer;\nuse serde::Deserialize;\n";
    let found = extract_imports(Lang::Rust, src).expect("extract");
    assert!(
        found.contains(&"store::fts".to_string()) || found.contains(&"store".to_string()),
        "{found:?}"
    );
    assert!(found.contains(&"tokenizer".to_string()), "{found:?}");
    // External crates appear too — resolution drops them later.
    assert!(
        found.contains(&"serde::Deserialize".to_string()),
        "{found:?}"
    );
}

#[test]
fn rust_pub_items_and_use_trees_truncate_at_brace() {
    let src = "pub use crate::errors::Error;\npub mod walker;\nuse crate::store::{\n    fts,\n    vector,\n};\nuse std::fmt::Write as _;\n";
    let found = extract_imports(Lang::Rust, src).expect("extract");
    assert!(found.contains(&"errors::Error".to_string()), "{found:?}");
    assert!(found.contains(&"walker".to_string()), "{found:?}");
    // The use-tree is cut at `::{` so only the module path remains.
    assert!(found.contains(&"store".to_string()), "{found:?}");
    // ` as ` renames keep only the imported path.
    assert!(found.contains(&"std::fmt::Write".to_string()), "{found:?}");
}

#[test]
fn typescript_imports_cover_quote_styles_and_bare_imports() {
    let src = "import { openDb } from './util/db';\nimport React from \"react\";\nimport 'reflect-metadata';\nimport * as path from 'path';\n";
    let found = extract_imports(Lang::Typescript, src).expect("extract");
    assert!(found.contains(&"./util/db".to_string()), "{found:?}");
    assert!(found.contains(&"react".to_string()), "{found:?}");
    assert!(found.contains(&"reflect-metadata".to_string()), "{found:?}");
    assert!(found.contains(&"path".to_string()), "{found:?}");
}

#[test]
fn typescript_relative_import_resolves_against_indexed_paths() {
    let indexed = paths(&["src/util/db.ts", "src/index.ts"]);
    // Unanchored: leading `./` is stripped and `util/db` suffix-matches.
    assert_eq!(
        PathIndex::new(&indexed).resolve("./util/db", None),
        Some("src/util/db.ts".to_string())
    );
    // Anchored: joined against the importer's directory, exact match.
    assert_eq!(
        PathIndex::new(&indexed).resolve("./util/db", Some("src/index.ts")),
        Some("src/util/db.ts".to_string())
    );
}

#[test]
fn relative_parent_modules_anchor_to_the_importing_file() {
    let indexed = paths(&["src/util/db.ts", "lib/util/db.ts"]);
    // Without an importer the stripped suffix `util/db` is ambiguous.
    assert_eq!(PathIndex::new(&indexed).resolve("../util/db", None), None);
    // Anchored to the importer, `..` walks from `src/views` up into `src`.
    assert_eq!(
        PathIndex::new(&indexed).resolve("../util/db", Some("src/views/page.ts")),
        Some("src/util/db.ts".to_string())
    );
    // `..` escaping the repo root cannot resolve.
    assert_eq!(
        PathIndex::new(&indexed).resolve("../../escape", Some("src/a.ts")),
        None
    );
}

#[test]
fn javascript_require_extracts_string_literals_only() {
    let src = "'use strict';\nconst legacy = require('./legacy');\nconst fs = require(\"fs\");\nconst dynamic = require(pluginName);\n";
    let found = extract_imports(Lang::Javascript, src).expect("extract");
    assert!(found.contains(&"./legacy".to_string()), "{found:?}");
    assert!(found.contains(&"fs".to_string()), "{found:?}");
    // Non-literal require() arguments are dropped, not guessed at.
    assert!(!found.iter().any(|m| m == "pluginName"), "{found:?}");
}

#[test]
fn javascript_require_resolves_relative_module() {
    let indexed = paths(&["src/legacy.js", "src/index.js"]);
    assert_eq!(
        PathIndex::new(&indexed).resolve("./legacy", None),
        Some("src/legacy.js".to_string())
    );
    assert_eq!(
        PathIndex::new(&indexed).resolve("./legacy", Some("src/index.js")),
        Some("src/legacy.js".to_string())
    );
}

#[test]
fn python_import_and_from_yield_dotted_modules() {
    let src = "import os\nimport numpy as np\nfrom app.models import User\nfrom collections import OrderedDict\nimport sys, json\n";
    let found = extract_imports(Lang::Python, src).expect("extract");
    for module in ["os", "numpy", "app.models", "collections", "sys", "json"] {
        assert!(
            found.contains(&module.to_string()),
            "missing {module}: {found:?}"
        );
    }
}

#[test]
fn python_dotted_module_resolves_to_file_or_package() {
    let indexed = paths(&["app/models.py", "app/views.py", "app/api/__init__.py"]);
    assert_eq!(
        PathIndex::new(&indexed).resolve("app.models", None),
        Some("app/models.py".to_string())
    );
    // Packages resolve via their `__init__.py` entry file.
    assert_eq!(
        PathIndex::new(&indexed).resolve("app.api", None),
        Some("app/api/__init__.py".to_string())
    );
    // External package: no local candidate.
    assert_eq!(PathIndex::new(&indexed).resolve("django.db", None), None);
}

#[test]
fn go_single_and_block_imports_extract_quoted_paths() {
    let src =
        "package main\n\nimport (\n\t\"fmt\"\n\tutil \"myrepo/pkg/util\"\n)\n\nimport \"os\"\n";
    let found = extract_imports(Lang::Go, src).expect("extract");
    for module in ["fmt", "myrepo/pkg/util", "os"] {
        assert!(
            found.contains(&module.to_string()),
            "missing {module}: {found:?}"
        );
    }
}

#[test]
fn go_module_prefixed_import_resolves_to_package_entry_file() {
    let indexed = paths(&["pkg/util/util.go", "pkg/util/helpers.go", "cmd/main.go"]);
    // Pinned Go rule: the package directory's entry file `<dir>/<dirname>.go`
    // stands in for the package, and one leading module-prefix segment
    // (`myrepo/`) is tolerated when the full path finds no candidate.
    assert_eq!(
        PathIndex::new(&indexed).resolve("myrepo/pkg/util", None),
        Some("pkg/util/util.go".to_string())
    );
    // A repo-relative package path resolves directly.
    assert_eq!(
        PathIndex::new(&indexed).resolve("pkg/util", None),
        Some("pkg/util/util.go".to_string())
    );
    // Stdlib package: no local candidate.
    assert_eq!(PathIndex::new(&indexed).resolve("fmt", None), None);
}

#[test]
fn resolve_matches_unique_suffix_and_skips_ambiguous() {
    let indexed = paths(&["src/store/fts.rs", "src/store/mod.rs", "src/eval/mod.rs"]);
    assert_eq!(
        PathIndex::new(&indexed).resolve("store::fts", None),
        Some("src/store/fts.rs".to_string())
    );
    assert_eq!(PathIndex::new(&indexed).resolve("mod", None), None); // ambiguous suffix (two mod.rs)
    assert_eq!(PathIndex::new(&indexed).resolve("serde", None), None); // external, no match
}

#[test]
fn resolve_requires_segment_aligned_suffixes() {
    let indexed = paths(&["src/bookstore/fts.rs"]);
    // `store/fts` is a substring suffix but not segment-aligned: no match.
    assert_eq!(PathIndex::new(&indexed).resolve("store::fts", None), None);
    assert_eq!(
        PathIndex::new(&indexed).resolve("bookstore::fts", None),
        Some("src/bookstore/fts.rs".to_string())
    );
}

#[test]
fn resolve_strips_dir_entry_files_to_their_directory() {
    let indexed = paths(&["src/store/mod.rs"]);
    // `mod.rs` stands in for its directory, so the bare module name matches.
    assert_eq!(
        PathIndex::new(&indexed).resolve("store", None),
        Some("src/store/mod.rs".to_string())
    );
}

#[test]
fn extraction_dedupes_preserving_first_seen_order() {
    let src = "use crate::store::fts;\nuse crate::store::fts;\nmod tokenizer;\n";
    let found = extract_imports(Lang::Rust, src).expect("extract");
    assert_eq!(
        found,
        vec!["store::fts".to_string(), "tokenizer".to_string()]
    );
}

#[test]
fn empty_source_yields_no_imports() {
    for lang in [
        Lang::Rust,
        Lang::Typescript,
        Lang::Javascript,
        Lang::Python,
        Lang::Go,
    ] {
        assert!(extract_imports(lang, "").expect("extract").is_empty());
    }
}

/// Mutation guard for the Go-prefix-retry match guard at
/// `src/graph/imports.rs:157`
/// (`None if module.contains('/') && !module.starts_with('.')`).
///
/// The guard restricts the "drop one leading segment and retry" tolerance
/// to slash-bearing, non-dotted module strings (Go module prefixes). A
/// DOTTED Python module like `a.b.c` has NO literal slash, so the guard is
/// false and resolution must STOP at the first `by_suffix` miss, returning
/// `None` — even though its normalized fragment `a/b/c` would, after
/// dropping the leading `a/`, suffix-match the indexed `x/b/c.py` on `b/c`.
///
/// Two surviving mutants both make this wrongly resolve to `Some`:
///   * guard → `true` (line 157:21): retry runs for the dotted module.
///   * `&&` → `||` (line 157:42): `contains('/')` is false but
///     `!starts_with('.')` is true, so `false || true` enables the retry.
///
/// The original guard yields `false && true = false`, so it returns
/// `None`. Asserting `None` kills BOTH survivors — each would instead
/// surface `Some("x/b/c.py")` via the unwarranted retry.
#[test]
fn dotted_module_does_not_trigger_go_prefix_retry() {
    // `a/b/c` has no full-fragment candidate; `b/c` does (via x/b/c.py).
    let indexed = paths(&["x/b/c.py"]);
    let idx = PathIndex::new(&indexed);

    // Sanity: the retry target `b/c` IS resolvable on its own, so a `Some`
    // here can only come from the guard wrongly enabling the retry.
    assert_eq!(idx.resolve("b.c", None), Some("x/b/c.py".to_string()));

    // The dotted three-segment module must NOT borrow the Go-prefix retry.
    assert_eq!(
        idx.resolve("a.b.c", None),
        None,
        "dotted (slash-less) module must not trigger the Go module-prefix retry",
    );
}
