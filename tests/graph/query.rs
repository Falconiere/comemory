use qwick_memory::config::paths::Paths;
use qwick_memory::graph::Graph;
use qwick_memory::memory::{Kind, MemoryStore};

use super::common;

#[test]
fn neighbors_by_repo_returns_empty_for_unknown_repo() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let g = Graph::open(paths.graph_dir()).unwrap();
    assert!(g.neighbors_by_repo("unknown").unwrap().is_empty());
}

#[test]
fn neighbors_by_repo_isolates_per_repo() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let a = store
        .save("alpha body", Kind::Note, "repo-a", &[], "x", 3)
        .unwrap();
    let b = store
        .save("bravo body", Kind::Note, "repo-b", &[], "x", 3)
        .unwrap();

    let g = Graph::open(paths.graph_dir()).unwrap();
    g.upsert_memory(&a).unwrap();
    g.upsert_memory(&b).unwrap();

    let ra = g.neighbors_by_repo("repo-a").unwrap();
    let rb = g.neighbors_by_repo("repo-b").unwrap();
    assert_eq!(ra, vec![a.frontmatter.id]);
    assert_eq!(rb, vec![b.frontmatter.id]);
}

#[test]
fn supersedes_edge_persists() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let a = store
        .save("a body", Kind::Decision, "r", &[], "x", 3)
        .unwrap();
    let b = store
        .save("b body", Kind::Decision, "r", &[], "x", 3)
        .unwrap();
    let g = Graph::open(paths.graph_dir()).unwrap();
    g.upsert_memory(&a).unwrap();
    g.upsert_memory(&b).unwrap();
    g.add_supersedes(&b.frontmatter.id, &a.frontmatter.id)
        .unwrap();
}

#[test]
fn supersedes_chain_walks_multiple_hops() {
    // Build a -> b -> c (b supersedes a, c supersedes b) and verify
    // `supersedes_chain(c, 5)` reaches both `b` (1 hop) and `a` (2 hops).
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let a = store
        .save("chain a", Kind::Decision, "r", &[], "x", 3)
        .unwrap();
    let b = store
        .save("chain b", Kind::Decision, "r", &[], "x", 3)
        .unwrap();
    let c = store
        .save("chain c", Kind::Decision, "r", &[], "x", 3)
        .unwrap();
    let g = Graph::open(paths.graph_dir()).unwrap();
    g.upsert_memory(&a).unwrap();
    g.upsert_memory(&b).unwrap();
    g.upsert_memory(&c).unwrap();
    g.add_supersedes(&b.frontmatter.id, &a.frontmatter.id)
        .unwrap();
    g.add_supersedes(&c.frontmatter.id, &b.frontmatter.id)
        .unwrap();

    let ids = g.supersedes_chain(&c.frontmatter.id, 5).unwrap();
    assert!(
        ids.contains(&b.frontmatter.id),
        "expected 1-hop b in chain: {ids:?}"
    );
    assert!(
        ids.contains(&a.frontmatter.id),
        "expected 2-hop a in chain: {ids:?}"
    );
    assert_eq!(ids.len(), 2, "exactly two hops expected: {ids:?}");
}

#[test]
fn supersedes_chain_empty_for_isolated_memory() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let m = store
        .save("isolated", Kind::Note, "r", &[], "x", 3)
        .unwrap();
    let g = Graph::open(paths.graph_dir()).unwrap();
    g.upsert_memory(&m).unwrap();
    assert!(g.supersedes_chain(&m.frontmatter.id, 3).unwrap().is_empty());
}

#[test]
fn conflicts_of_returns_outgoing_neighbours() {
    // We have no public `add_conflicts_with` helper; insert the edge directly
    // via a one-shot Cypher MERGE so the test stays self-contained.
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let a = store
        .save("conflict a", Kind::Decision, "r", &[], "x", 3)
        .unwrap();
    let b = store
        .save("conflict b", Kind::Decision, "r", &[], "x", 3)
        .unwrap();
    let g = Graph::open(paths.graph_dir()).unwrap();
    g.upsert_memory(&a).unwrap();
    g.upsert_memory(&b).unwrap();

    let conn = g.conn().unwrap();
    let cypher = format!(
        "MATCH (x:Memory {{id: '{}'}}), (y:Memory {{id: '{}'}}) MERGE (x)-[:ConflictsWith]->(y)",
        a.frontmatter.id, b.frontmatter.id
    );
    conn.query(&cypher).unwrap();

    let out = g.conflicts_of(&a.frontmatter.id).unwrap();
    assert_eq!(out, vec![b.frontmatter.id.clone()]);

    // Reverse direction is empty — the edge is directional.
    assert!(g.conflicts_of(&b.frontmatter.id).unwrap().is_empty());
}

#[test]
fn conflicts_of_unknown_memory_is_empty() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let g = Graph::open(paths.graph_dir()).unwrap();
    assert!(g.conflicts_of("nonexistent").unwrap().is_empty());
}

use super::graph_fixture;

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

#[test]
fn expand_returns_one_hop_for_known_memory() {
    let fx = graph_fixture::build();
    let seed = format!("m:{}", fx.primary_id);
    let payload = fx.graph.expand_neighbors(&seed, 1).expect("expand");
    let ids: std::collections::BTreeSet<_> = payload.nodes.iter().map(|n| n.id.clone()).collect();
    assert!(ids.contains(&seed));
    assert!(ids.contains("r:qwick-backend"));
    assert!(ids.contains("a:falconiere"));
    assert!(ids.contains("t:database") || ids.contains("t:postgres"));
}

#[test]
fn expand_unknown_id_returns_empty_payload() {
    let fx = graph_fixture::build();
    let payload = fx.graph.expand_neighbors("m:zzzzzzzz", 1).expect("expand");
    assert!(payload.nodes.is_empty());
    assert!(payload.edges.is_empty());
}

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
