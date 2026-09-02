#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Document-domain journey over `/api/v1` — the HTTP twin of
//! `tests/cli_scenario_documents.rs`: register a docs source (job) → list
//! sources → `find --domain document` → unindex → sources/find both go
//! empty, against a real `comemory serve` and the real docs fixtures.

#[path = "common/docs_fixtures.rs"]
mod docs_fixtures;
#[path = "common/serve_bin.rs"]
mod serve_bin;

use serde_json::json;
use serve_bin::ServeHome;

#[test]
fn index_find_unindex_document_loop() {
    let tmp = tempfile::TempDir::new().expect("workspace");
    let workspace = tmp.path().to_str().expect("utf8").to_string();
    let srv = ServeHome::with_args(&["--allow-path", &workspace]);

    let docs = docs_fixtures::seed(tmp.path());
    let docs_s = docs.to_str().expect("utf8").to_string();

    let indexed = srv.job("/sources", &json!({ "path": [docs_s] }));
    let sources = indexed["sources"].as_array().expect("sources");
    assert_eq!(sources.len(), 1, "{indexed}");
    assert_eq!(
        sources[0]["indexed"].as_u64(),
        Some(docs_fixtures::FIXTURE_COUNT as u64),
        "{indexed}"
    );

    let listed = srv.get("/sources");
    let rows = listed.as_array().expect("sources array");
    assert_eq!(rows.len(), 1, "{listed}");
    assert!(
        rows[0]["indexed"].as_u64().expect("indexed") >= 1,
        "{listed}"
    );

    let found = srv.get_q("/find", &[("query", "Homebrew"), ("domain", "document")]);
    let hits = found["hits"].as_array().expect("hits");
    assert!(
        hits.iter().any(|h| h["domain"] == "document"),
        "guide.md mentions Homebrew: {found}"
    );

    // Path form of `DELETE /sources/{target}?confirm=true` (spec §6): the
    // registered path is a single URL path segment, so its `/` separators
    // are percent-encoded to `%2F` rather than left to split the route.
    let target = docs_s.replace('/', "%2F");
    let unindexed = srv.delete(&format!("/sources/{target}?confirm=true"));
    assert_eq!(
        unindexed["documents_removed"].as_u64(),
        Some(docs_fixtures::FIXTURE_COUNT as u64),
        "{unindexed}"
    );

    let after = srv.get("/sources");
    let after_rows = after.as_array().expect("sources array");
    assert!(
        after_rows.is_empty(),
        "unindex must drop the source: {after}"
    );

    let gone = srv.get_q("/find", &[("query", "Homebrew"), ("domain", "document")]);
    let gone_hits = gone["hits"].as_array().expect("hits");
    assert!(
        gone_hits.iter().all(|h| h["domain"] != "document"),
        "document hits must disappear: {gone}"
    );
}
