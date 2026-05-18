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
