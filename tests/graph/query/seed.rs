//! Tests for `Graph::seed_memory_layer` and `Graph::seed_all`.

use crate::graph_fixture;

#[test]
fn seed_all_includes_file_symbol_and_cross_layer_edges() {
    let fx = graph_fixture::build();
    let payload = fx.graph.seed_all().expect("seed all");
    let kinds: std::collections::BTreeSet<_> =
        payload.nodes.iter().map(|n| n.kind.as_str()).collect();
    assert!(kinds.contains("File"));
    assert!(kinds.contains("Symbol"));

    let edge_kinds: std::collections::BTreeSet<_> =
        payload.edges.iter().map(|e| e.kind.as_str()).collect();
    assert!(edge_kinds.contains("ReferencesFile"));
    assert!(edge_kinds.contains("ReferencesSymbol"));
    assert!(edge_kinds.contains("DefinedIn"));
    assert!(edge_kinds.contains("InRepo"));
}

#[test]
fn seed_memory_layer_returns_memory_repo_author_tag() {
    let fx = graph_fixture::build();
    let payload = fx.graph.seed_memory_layer().expect("seed");
    let kinds: std::collections::BTreeSet<_> =
        payload.nodes.iter().map(|n| n.kind.as_str()).collect();
    assert!(kinds.contains("Memory"));
    assert!(kinds.contains("Repo"));
    assert!(kinds.contains("Author"));
    assert!(kinds.contains("Tag"));
    assert!(!kinds.contains("File"));
    assert!(!kinds.contains("Symbol"));

    let memories = payload.nodes.iter().filter(|n| n.kind == "Memory").count();
    assert_eq!(memories, 3);

    // Verify node ids reference the fixture's known ids and repo.
    let mem_ids: std::collections::BTreeSet<_> = payload
        .nodes
        .iter()
        .filter(|n| n.kind == "Memory")
        .map(|n| n.id.as_str())
        .collect();
    assert!(
        mem_ids.contains(format!("m:{}", fx.primary_id).as_str()),
        "primary memory missing: {mem_ids:?}"
    );
    assert!(
        mem_ids.contains(format!("m:{}", fx.superseded_id).as_str()),
        "superseded memory missing: {mem_ids:?}"
    );
    assert!(
        mem_ids.contains(format!("m:{}", fx.conflict_id).as_str()),
        "conflict memory missing: {mem_ids:?}"
    );

    // Repo and tag nodes should carry the fixture's known labels.
    let repo_node = payload
        .nodes
        .iter()
        .find(|n| n.kind == "Repo")
        .expect("repo node");
    assert_eq!(repo_node.label, fx.repo);

    let tag_node = payload
        .nodes
        .iter()
        .find(|n| n.kind == "Tag" && n.label == fx.tag)
        .expect("tag node for 'database'");
    assert_eq!(tag_node.id, format!("t:{}", fx.tag));

    // Paths must be valid (data dir was created by build()).
    assert!(fx.paths.graph_dir().exists());

    // file_qualified and symbol_qualified are fixture metadata used by
    // expand-layer tests; confirm they have the expected namespaced form.
    assert!(
        fx.file_qualified.contains(':'),
        "file_qualified must be repo:path form: {}",
        fx.file_qualified
    );
    assert!(
        fx.symbol_qualified.contains(':'),
        "symbol_qualified must be repo:path:symbol form: {}",
        fx.symbol_qualified
    );
}
