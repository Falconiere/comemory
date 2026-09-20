#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

use std::path::Path;

use comemory::domains::architecture::cluster::{ident, key_for, summary_from_readme};

#[test]
fn a_key_is_the_leading_directory_segments_at_the_requested_depth() {
    assert_eq!(key_for("src/domains/graph/query.rs", 2), "src/domains");
    assert_eq!(
        key_for("src/domains/graph/query.rs", 3),
        "src/domains/graph"
    );
    assert_eq!(key_for("src/cli.rs", 2), "src");
    assert_eq!(key_for("build.rs", 2), "build.rs");
}

#[test]
fn an_id_is_renderer_safe_and_stable() {
    assert_eq!(ident("src/domains/graph"), "src_domains_graph");
    assert_eq!(ident("2026-reports"), "c_2026_reports");
    assert_eq!(ident("a"), "a");
    let long = ident(&"x/".repeat(80));
    assert!(long.len() <= 64 && !long.ends_with('_'), "{long}");
}

#[test]
fn a_summary_is_seeded_from_this_repository_own_folder_readme() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let summary = summary_from_readme(root, "src/domains/capture").expect("capture has a README");
    assert!(
        summary.starts_with("What belongs here:"),
        "unexpected seed: {summary:?}"
    );
    assert!(summary.len() <= 280, "{}", summary.len());
    assert!(summary_from_readme(root, "src/domains/nope").is_none());
}
