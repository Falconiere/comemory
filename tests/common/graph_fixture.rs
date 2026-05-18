//! Build a deterministic in-memory graph for handler tests.
//!
//! Memory ids are content-derived, so each call to `build()` returns the
//! same ids for the same bodies. The `Fixture` struct exposes the
//! computed ids so tests can reference them without hardcoding hashes.

use std::path::PathBuf;

use qwick_memory::config::paths::Paths;
use qwick_memory::graph::Graph;
use qwick_memory::memory::{Kind, MemoryStore};
use tempfile::TempDir;

/// Owning handle: drop after the test to clean up the temp directory.
pub struct Fixture {
    pub paths: Paths,
    pub graph: Graph,
    pub primary_id: String,
    pub superseded_id: String,
    pub conflict_id: String,
    pub repo: String,
    pub tag: String,
    pub file_qualified: String,
    pub symbol_qualified: String,
    _tmp: TempDir,
}

/// Build the canonical fixture and return open handles.
pub fn build() -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(PathBuf::from(tmp.path()));
    paths.ensure_dirs().expect("ensure_dirs");
    let graph = Graph::open(paths.graph_dir()).expect("graph open");

    let store = MemoryStore::new(paths.clone());

    let primary = store
        .save(
            "# primary memory\n",
            Kind::Decision,
            "qwick-backend",
            &["database".to_string(), "postgres".to_string()],
            "falconiere",
            4,
        )
        .expect("save primary");
    graph.upsert_memory(&primary).expect("upsert primary");

    let superseded = store
        .save(
            "old superseded memory\n",
            Kind::Decision,
            "qwick-backend",
            &[],
            "falconiere",
            3,
        )
        .expect("save superseded");
    graph.upsert_memory(&superseded).expect("upsert superseded");

    let conflicting = store
        .save(
            "conflicting memory body\n",
            Kind::Decision,
            "qwick-backend",
            &[],
            "falconiere",
            3,
        )
        .expect("save conflicting");
    graph
        .upsert_memory(&conflicting)
        .expect("upsert conflicting");

    graph
        .add_supersedes(&primary.frontmatter.id, &superseded.frontmatter.id)
        .expect("add supersedes");

    // ConflictsWith — direct query (no public helper on Graph)
    {
        let conn = graph.conn().expect("conn");
        let cypher = format!(
            "MATCH (a:Memory {{id: '{}'}}), (b:Memory {{id: '{}'}}) \
             MERGE (a)-[:ConflictsWith]->(b)",
            primary.frontmatter.id, conflicting.frontmatter.id,
        );
        conn.query(&cypher).expect("conflicts edge");
    }

    let file_qualified = "qwick-backend:src/db.rs".to_string();
    let symbol_qualified = "qwick-backend:src/db.rs:open".to_string();
    let zero_hash = "0".repeat(64);

    graph
        .upsert_file(&file_qualified, "qwick-backend", "src/db.rs", &zero_hash)
        .expect("upsert file");
    graph
        .upsert_symbol(
            &symbol_qualified,
            "open",
            "fn",
            "rust",
            &zero_hash,
            &file_qualified,
        )
        .expect("upsert symbol");
    graph
        .add_references_file(&primary.frontmatter.id, &file_qualified)
        .expect("ref file");
    graph
        .add_references_symbol(&primary.frontmatter.id, &symbol_qualified)
        .expect("ref symbol");

    Fixture {
        paths,
        graph,
        primary_id: primary.frontmatter.id.clone(),
        superseded_id: superseded.frontmatter.id.clone(),
        conflict_id: conflicting.frontmatter.id.clone(),
        repo: "qwick-backend".into(),
        tag: "database".into(),
        file_qualified,
        symbol_qualified,
        _tmp: tmp,
    }
}
