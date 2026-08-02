//! `--only` / `--path` domain-scope tests for `comemory search` — split
//! from `cli__search.rs`/`cli__search_2.rs` (same precedent: a cohesive
//! flag family gets its own mirror file). Real fixtures throughout: the
//! document corpus comes from `common::docs_fixtures::seed` indexed
//! through the real `comemory index` binary, memories through `save`.

#[path = "common/docs_fixtures.rs"]
mod docs_fixtures;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::tempdir;

/// Index the real document fixtures (`guide.md`, `changelog.txt`,
/// `page.html`, `data.csv`) under `home`'s data dir.
fn index_docs_fixtures(home: &std::path::Path, workspace: &std::path::Path) {
    let docs = docs_fixtures::seed(workspace);
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .args(["index", docs.to_str().expect("utf8 path")])
        .assert()
        .success();
}

/// Save one memory through the real CLI.
fn save_memory(home: &std::path::Path, body: &str) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .args(["save", "--kind", "note", body])
        .assert()
        .success();
}

/// Run `comemory search --json` with extra args, return the parsed
/// output. Access tracking is disabled so repeated runs never mutate
/// ranking inputs between calls (mirrors `cli__search.rs::search_json`).
fn search_json(home: &std::path::Path, query: &str, extra: &[&str]) -> Value {
    let mut args = vec!["search", query, "--json"];
    args.extend_from_slice(extra);
    let assert = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .env("COMEMORY_DISABLE_ACCESS_TRACKING", "true")
        .args(&args)
        .assert()
        .success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("json ({e}): {out}"))
}

/// Run a `comemory search` expected to fail, returning `(exit code, stderr)`.
fn reject(home: &std::path::Path, query: &str, extra: &[&str]) -> (Option<i32>, String) {
    let mut args = vec!["search", query];
    args.extend_from_slice(extra);
    let assert = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .args(&args)
        .assert()
        .failure();
    let out = assert.get_output();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn only_document_returns_document_hits_from_the_indexed_corpus() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    index_docs_fixtures(home.path(), workspace.path());
    // Every glob test below assumes this exact corpus shape; pin it.
    assert_eq!(docs_fixtures::FIXTURE_COUNT, 4);

    let v = search_json(home.path(), "comemory", &["--only", "document"]);
    let items = v["items"].as_array().expect("items array");
    assert!(!items.is_empty(), "expected document hits, got: {v}");
    for item in items {
        assert_eq!(item["domain"], "document", "row: {item}");
        assert!(item["document_id"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(item["citation"]["path"].as_str().is_some());
    }
}

#[test]
fn only_memory_comma_code_is_a_usage_error() {
    // Code doesn't join unified search yet — before this fix, `--only
    // memory,code` silently ran the memory-only leg and dropped the
    // `code` half of the request without a word. Any --only scope that
    // names `code` must now be rejected instead.
    let home = tempdir().expect("home");
    let (code, stderr) = reject(home.path(), "comemory", &["--only", "memory,code"]);
    assert_eq!(code, Some(64), "must exit EX_USAGE: {stderr}");
    assert!(
        stderr.contains("search-code"),
        "error must point at search-code: {stderr}"
    );
}

#[test]
fn only_memory_comma_document_is_a_usage_error() {
    // Before this fix, `--only memory,document` routed through
    // `run_memory` (triggered by the Memory bit alone), which never reads
    // `Filters.domains` — the document half of the request silently
    // vanished with no error and no partial results. Any --only scope
    // naming both memory and document must now be rejected instead.
    let home = tempdir().expect("home");
    let (code, stderr) = reject(home.path(), "comemory", &["--only", "memory,document"]);
    assert_eq!(code, Some(64), "must exit EX_USAGE: {stderr}");
    assert!(
        stderr.contains("memory") && stderr.contains("document"),
        "error must name both domains: {stderr}"
    );
}

#[test]
fn only_code_alone_is_a_usage_error_not_a_silent_empty_result() {
    // Before this fix, `--only code` alone routed to the document-only
    // path, which excludes the code domain too and just returns an
    // empty page with exit 0 — no signal that code was never searched.
    let home = tempdir().expect("home");
    let (code, stderr) = reject(home.path(), "comemory", &["--only", "code"]);
    assert_eq!(code, Some(64), "must exit EX_USAGE: {stderr}");
    assert!(
        stderr.contains("search-code"),
        "error must point at search-code: {stderr}"
    );
}

#[test]
fn only_code_comma_document_is_a_usage_error() {
    let home = tempdir().expect("home");
    let (code, stderr) = reject(home.path(), "comemory", &["--only", "code,document"]);
    assert_eq!(code, Some(64), "must exit EX_USAGE: {stderr}");
    assert!(
        stderr.contains("search-code"),
        "error must point at search-code: {stderr}"
    );
}

#[test]
fn invalid_only_value_is_a_usage_error_listing_domains() {
    let home = tempdir().expect("home");
    let (code, stderr) = reject(home.path(), "q", &["--only", "bogus"]);
    assert!(code.is_some_and(|c| c != 0), "must fail: {stderr}");
    for domain in ["memory", "document", "code"] {
        assert!(
            stderr.contains(domain),
            "error must list valid domain `{domain}`: {stderr}"
        );
    }
}

#[test]
fn only_code_with_kind_is_a_usage_error() {
    let home = tempdir().expect("home");
    let (code, stderr) = reject(home.path(), "q", &["--only", "code", "--kind", "decision"]);
    assert_eq!(
        code,
        Some(64),
        "contradictory filters exit EX_USAGE: {stderr}"
    );
    assert!(
        stderr.contains("--kind") && stderr.contains("only"),
        "error must name both the kind value and --only: {stderr}"
    );
}

#[test]
fn path_glob_narrows_document_results_to_the_matching_path() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    index_docs_fixtures(home.path(), workspace.path());

    // "comemory" matches guide.md, changelog.txt, and page.html.
    let unfiltered = search_json(home.path(), "comemory", &["--only", "document"]);
    let unfiltered_paths: Vec<String> = unfiltered["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["citation"]["path"].as_str().expect("path").to_string())
        .collect();
    assert!(
        unfiltered_paths.len() >= 2,
        "need at least two matches for the narrowing to be meaningful: {unfiltered_paths:?}"
    );

    let filtered = search_json(
        home.path(),
        "comemory",
        &["--only", "document", "--path", "*.md"],
    );
    let items = filtered["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "only guide.md must survive: {filtered}");
    assert_eq!(items[0]["citation"]["path"], "guide.md");
}

#[test]
fn path_glob_entries_are_ored_together() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    index_docs_fixtures(home.path(), workspace.path());

    // "search" matches guide.md, page.html, and data.csv but not
    // changelog.txt; two globs must OR to the .md + .csv subset.
    let v = search_json(
        home.path(),
        "search",
        &["--only", "document", "--path", "*.md", "--path", "*.csv"],
    );
    let mut paths: Vec<String> = v["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["citation"]["path"].as_str().expect("path").to_string())
        .collect();
    paths.sort();
    assert_eq!(
        paths,
        vec!["data.csv".to_string(), "guide.md".to_string()],
        "OR of both globs, page.html excluded: {v}"
    );
}

#[test]
fn path_flag_has_no_effect_on_memory_results() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    index_docs_fixtures(home.path(), workspace.path());
    save_memory(home.path(), "widget rollout advisory note");

    let plain = search_json(home.path(), "widget", &[]);
    let with_path = search_json(home.path(), "widget", &["--path", "docs/**"]);
    assert_eq!(
        plain["hits"], with_path["hits"],
        "--path must not change memory results at all"
    );
}

#[test]
fn memory_only_query_output_is_unchanged_by_the_only_and_path_flags_existing() {
    let home = tempdir().expect("home");
    save_memory(home.path(), "postgres advisory lock migration ordering");

    let v = search_json(home.path(), "advisory lock", &[]);
    let hits = v["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty(), "got: {v}");
    for hit in hits {
        // The pre-`--only` memory `Row` shape has no `domain`/`fused_score`
        // fields — s9 pins those. Their absence here is the regression
        // guard that this increment did not touch the memory row shape.
        assert!(hit.get("domain").is_none(), "row: {hit}");
        assert!(hit.get("fused_score").is_none(), "row: {hit}");
        assert!(hit.get("memory_id").is_some(), "row: {hit}");
    }
}
