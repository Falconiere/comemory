//! Tests for `Graph::search_nodes`.

use crate::graph_fixture;

#[test]
fn search_matches_tag_and_memory_id() {
    let fx = graph_fixture::build();

    let hits = fx.graph.search_nodes("database", 20).expect("search");
    assert!(hits
        .iter()
        .any(|n| n.kind == "Tag" && n.label == "database"));

    let prefix: String = fx.primary_id.chars().take(4).collect();
    let hits = fx.graph.search_nodes(&prefix, 20).expect("search");
    assert!(
        hits.iter()
            .any(|n| n.kind == "Memory" && n.id == format!("m:{}", fx.primary_id)),
        "expected memory id match for prefix `{}` in {:?}",
        prefix,
        hits,
    );
}

#[test]
fn search_respects_limit() {
    let fx = graph_fixture::build();
    let hits = fx.graph.search_nodes("a", 2).expect("search");
    assert!(hits.len() <= 2);
}
