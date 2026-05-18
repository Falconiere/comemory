use qwick::config::paths::Paths;
use qwick::graph::Graph;
use qwick::memory::{Kind, MemoryStore};

use super::common;

#[test]
fn upsert_memory_creates_repo_edge() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save(
            "Use Postgres for analytics",
            Kind::Decision,
            "myrepo",
            &["t1".into(), "t2".into()],
            "alice",
            4,
        )
        .unwrap();

    let g = Graph::open(paths.graph_dir()).unwrap();
    g.upsert_memory(&rec).unwrap();
    let ids = g.neighbors_by_repo("myrepo").unwrap();
    assert_eq!(ids, vec![rec.frontmatter.id]);
}

#[test]
fn upsert_memory_is_idempotent() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save("idempotent body", Kind::Note, "r", &["t".into()], "a", 3)
        .unwrap();

    let g = Graph::open(paths.graph_dir()).unwrap();
    g.upsert_memory(&rec).unwrap();
    g.upsert_memory(&rec).unwrap();

    let ids = g.neighbors_by_repo("r").unwrap();
    assert_eq!(
        ids.iter().filter(|id| **id == rec.frontmatter.id).count(),
        1,
        "duplicate InRepo edges from repeated upsert",
    );
}

#[test]
fn upsert_memory_escapes_single_quotes_in_repo_and_tags() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save(
            "body with tricky names",
            Kind::Discovery,
            "weird'repo",
            &["tag'with'quote".into()],
            "auth'or",
            2,
        )
        .unwrap();

    let g = Graph::open(paths.graph_dir()).unwrap();
    g.upsert_memory(&rec).unwrap();

    let ids = g.neighbors_by_repo("weird'repo").unwrap();
    assert_eq!(
        ids,
        vec![rec.frontmatter.id],
        "escaped repo lookup should round-trip",
    );
}

#[test]
fn add_relates_to_does_not_error_after_upsert() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let a = store
        .save("first body", Kind::Decision, "r", &[], "u", 3)
        .unwrap();
    let b = store
        .save("second body", Kind::Decision, "r", &[], "u", 3)
        .unwrap();

    let g = Graph::open(paths.graph_dir()).unwrap();
    g.upsert_memory(&a).unwrap();
    g.upsert_memory(&b).unwrap();
    g.add_relates_to(&a.frontmatter.id, &b.frontmatter.id, 0.75)
        .unwrap();
}

#[test]
fn upsert_file_and_symbol_round_trip() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let g = Graph::open(paths.graph_dir()).unwrap();

    g.upsert_file(
        "qwick-backend:src/db.rs",
        "qwick-backend",
        "src/db.rs",
        "deadbeef",
    )
    .unwrap();
    g.upsert_symbol(
        "qwick-backend:src/db.rs:run_migration",
        "run_migration",
        "function",
        "rust",
        "c0ffee",
        "qwick-backend:src/db.rs",
    )
    .unwrap();
    // Repeating must not error (MERGE semantics).
    g.upsert_file(
        "qwick-backend:src/db.rs",
        "qwick-backend",
        "src/db.rs",
        "deadbeef",
    )
    .unwrap();
    g.upsert_symbol(
        "qwick-backend:src/db.rs:run_migration",
        "run_migration",
        "function",
        "rust",
        "c0ffee",
        "qwick-backend:src/db.rs",
    )
    .unwrap();
}

#[test]
fn add_references_file_and_symbol_work_after_upserts() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save(
            "Body mentioning qwick-backend:src/db.rs:run_migration",
            Kind::Discovery,
            "qwick-backend",
            &[],
            "alice",
            3,
        )
        .unwrap();

    let g = Graph::open(paths.graph_dir()).unwrap();
    g.upsert_memory(&rec).unwrap();
    g.upsert_file(
        "qwick-backend:src/db.rs",
        "qwick-backend",
        "src/db.rs",
        "deadbeef",
    )
    .unwrap();
    g.upsert_symbol(
        "qwick-backend:src/db.rs:run_migration",
        "run_migration",
        "function",
        "rust",
        "c0ffee",
        "qwick-backend:src/db.rs",
    )
    .unwrap();

    g.add_references_file(&rec.frontmatter.id, "qwick-backend:src/db.rs")
        .unwrap();
    g.add_references_symbol(&rec.frontmatter.id, "qwick-backend:src/db.rs:run_migration")
        .unwrap();
}
