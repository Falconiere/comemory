use qwick::config::paths::Paths;
use qwick::graph::Graph;
use qwick::memory::{Kind, MemoryStore};

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
