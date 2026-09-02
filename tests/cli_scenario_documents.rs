#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Document-domain journey: index fixtures → sources → find → unindex.

#[path = "common/cli_bin.rs"]
mod cli_bin;
#[path = "common/docs_fixtures.rs"]
mod docs_fixtures;

use cli_bin::CliHome;

#[test]
fn index_find_unindex_document_loop() {
    let home = CliHome::new();
    let docs = docs_fixtures::seed(home.data_dir().parent().expect("parent"));
    let docs_s = docs.to_str().expect("utf8");

    let indexed = home.run_json(&["index", docs_s]);
    let sources = indexed["sources"].as_array().expect("sources");
    assert_eq!(sources.len(), 1, "{indexed}");
    assert_eq!(
        sources[0]["indexed"].as_u64(),
        Some(docs_fixtures::FIXTURE_COUNT as u64),
        "{indexed}"
    );

    let listed = home.run_json(&["sources"]);
    let rows = listed.as_array().expect("sources array");
    assert_eq!(rows.len(), 1, "{listed}");
    assert!(
        rows[0]["indexed"].as_u64().expect("indexed") >= 1,
        "{listed}"
    );

    let found = home.run_json(&["find", "Homebrew", "--domain", "document"]);
    let hits = found["hits"].as_array().expect("hits");
    assert!(
        hits.iter().any(|h| h["domain"] == "document"),
        "guide.md mentions Homebrew: {found}"
    );

    home.run_ok(&["unindex", docs_s]);

    let after = home.run_json(&["sources"]);
    let after_rows = after.as_array().expect("sources array");
    assert!(
        after_rows.is_empty(),
        "unindex must drop the source: {after}"
    );

    let gone = home.run_json(&["find", "Homebrew", "--domain", "document"]);
    let gone_hits = gone["hits"].as_array().expect("hits");
    assert!(
        gone_hits.iter().all(|h| h["domain"] != "document"),
        "document hits must disappear: {gone}"
    );
}
