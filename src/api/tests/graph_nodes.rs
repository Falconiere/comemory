#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for `comemory::api::graph_nodes` (console-api spec §5) over a REAL
//! indexed git repo: three Rust files under `src/`, `a.rs` importing `b.rs`,
//! walked by `api::index_code::run` so the `imports` edges, the `code_symbols`
//! rows and the materialized `rank_score` under assertion are the ones
//! production writes — no hand-seeded graph.
//!
//! Includes AC-9: `GET /graph/nodes/{id}/neighbors` returns exactly the rows
//! `comemory context` reports as `neighbors` for a memory citing that file.

use crate::test_common::{git_commit, git_repo};

use std::path::{Path, PathBuf};

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::prelude::Error;
use comemory::retrieval::bundle;
use comemory::retrieval::code_rerank::WorkingSet;
use comemory::store::connection;
use tempfile::TempDir;

/// Repo label every test in this file indexes under.
const REPO: &str = "demo";

/// The whole fixture: both temp dirs (kept alive), the resolved paths, the
/// default config, and the open connection every `Ctx` borrows.
struct Store {
    _workspace: TempDir,
    _home: TempDir,
    paths: Paths,
    cfg: Config,
    conn: rusqlite::Connection,
}

/// Build `<root>/import-repo`: `src/a.rs` declares `mod b;` (minting
/// `file:demo:src/a.rs --imports-> file:demo:src/b.rs`), plus an unrelated
/// `src/c.rs` so the node list has something to order.
fn build_import_repo(root: &Path) -> PathBuf {
    let repo = root.join("import-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[
            (
                "src/a.rs",
                "mod b;\n\npub fn alpha() {\n    b::beta();\n}\n",
            ),
            ("src/b.rs", "pub fn beta() {}\n"),
            ("src/c.rs", "pub fn gamma() {}\n"),
        ],
        "seed a + b + c",
    );
    repo
}

/// A fresh data-dir with [`build_import_repo`] indexed into it.
fn indexed_store() -> Store {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_import_repo(workspace.path());
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).expect("open db");
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::index_code::run(
            &mut ctx,
            api::index_code::Request {
                repo: REPO.to_string(),
                path: repo_root.to_str().expect("utf8 repo path").to_string(),
                mode: api::index_code::IndexMode::Incremental,
            },
        )
        .expect("index_code run");
    }
    Store {
        _workspace: workspace,
        _home: home,
        paths,
        cfg,
        conn,
    }
}

impl Store {
    /// A `Ctx` borrowing this store's connection.
    fn ctx(&mut self) -> Ctx<'_> {
        Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn)
    }

    /// Save one memory whose body cites `demo:src/a.rs` in backticks, so
    /// `cross_link` mints the `references_file` edge, and return its id.
    fn save_citing_a(&mut self) -> String {
        let req = api::save::Request {
            body: "The alpha entry point lives in `demo:src/a.rs` and calls beta.".to_string(),
            title: None,
            kind: Kind::Note,
            repo: REPO.to_string(),
            tags: Vec::new(),
            author: String::new(),
            quality: 3,
            supersedes: Vec::new(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        };
        let mut ctx = self.ctx();
        api::save::run(&mut ctx, req, false, None)
            .expect("save citing memory")
            .id
    }
}

/// A `ListRequest` for the whole corpus with an explicit sort.
fn list_req(sort: Option<&str>) -> api::graph_nodes::ListRequest {
    api::graph_nodes::ListRequest {
        repo: Some(REPO.to_string()),
        sort: sort.map(str::to_string),
        limit: None,
        offset: None,
    }
}

#[test]
fn list_defaults_to_pagerank_order_over_real_scores() {
    let mut store = indexed_store();
    let page = api::graph_nodes::list(&mut store.ctx(), list_req(None)).expect("list");

    assert_eq!(page.items.len(), 3, "one node per indexed file: {page:?}");
    assert_eq!(page.total, Some(3));
    assert!(!page.has_more);
    assert!(
        page.items.iter().any(|n| n.rank > 0.0),
        "index-code must have materialized a real PageRank: {page:?}"
    );
    for pair in page.items.windows(2) {
        assert!(
            pair[0].rank >= pair[1].rank,
            "pagerank order is descending: {page:?}"
        );
    }
}

#[test]
fn list_sort_path_orders_by_node_id() {
    let mut store = indexed_store();
    let page = api::graph_nodes::list(&mut store.ctx(), list_req(Some("path"))).expect("list");

    let ids: Vec<&str> = page.items.iter().map(|n| n.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "path order is node-id ascending");
    assert_eq!(ids.first().copied(), Some("file:demo:src/a.rs"));
}

#[test]
fn list_rejects_an_unknown_sort() {
    let mut store = indexed_store();
    let err = api::graph_nodes::list(&mut store.ctx(), list_req(Some("rank"))).unwrap_err();
    assert!(matches!(err, Error::BadRequest(_)), "got {err:?}");
}

#[test]
fn list_pages_with_limit_and_offset() {
    let mut store = indexed_store();
    let mut req = list_req(Some("path"));
    req.limit = Some(2);
    let first = api::graph_nodes::list(&mut store.ctx(), req).expect("first page");
    assert_eq!(first.items.len(), 2);
    assert!(first.has_more);

    let mut req = list_req(Some("path"));
    req.limit = Some(2);
    req.offset = Some(2);
    let second = api::graph_nodes::list(&mut store.ctx(), req).expect("second page");
    assert_eq!(second.items.len(), 1);
    assert!(!second.has_more);
}

#[test]
fn detail_accepts_the_prefixed_and_bare_id_forms_and_a_scoped_path() {
    let mut store = indexed_store();
    let prefixed =
        api::graph_nodes::detail(&mut store.ctx(), "file:demo:src/a.rs", None).expect("prefixed");
    let bare = api::graph_nodes::detail(&mut store.ctx(), "demo:src/a.rs", None).expect("bare");
    let scoped =
        api::graph_nodes::detail(&mut store.ctx(), "src/a.rs", Some(REPO)).expect("scoped path");

    assert_eq!(prefixed.node.id, "file:demo:src/a.rs");
    assert_eq!(bare.node.id, prefixed.node.id);
    assert_eq!(scoped.node.id, prefixed.node.id);
    assert!(
        prefixed.top_symbols.iter().any(|s| s.symbol == "alpha"),
        "top_symbols must carry the file's real symbols: {:?}",
        prefixed.top_symbols
    );
    let alpha = prefixed
        .top_symbols
        .iter()
        .find(|s| s.symbol == "alpha")
        .expect("alpha row");
    assert_eq!(alpha.lang, "rust");
    assert!(alpha.line_end >= alpha.line_start);
}

#[test]
fn detail_is_not_found_for_a_file_with_no_indexed_symbols() {
    let mut store = indexed_store();
    let err =
        api::graph_nodes::detail(&mut store.ctx(), "file:demo:src/missing.rs", None).unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "got {err:?}");
}

#[test]
fn detail_rejects_an_unparsable_id_without_a_repo_scope() {
    let mut store = indexed_store();
    let err = api::graph_nodes::detail(&mut store.ctx(), "src/a.rs", None).unwrap_err();
    assert!(matches!(err, Error::BadRequest(_)), "got {err:?}");
}

#[test]
fn detail_lists_the_memory_citing_the_file_and_agrees_with_the_node_count() {
    let mut store = indexed_store();
    let id = store.save_citing_a();

    let detail =
        api::graph_nodes::detail(&mut store.ctx(), "file:demo:src/a.rs", None).expect("detail");
    assert_eq!(detail.cited_by.len(), 1, "cited_by: {:?}", detail.cited_by);
    assert_eq!(detail.cited_by[0].id, id);
    assert!(
        detail.cited_by[0]
            .title
            .starts_with("The alpha entry point"),
        "title: {:?}",
        detail.cited_by[0].title
    );
    assert_eq!(
        detail.node.memories,
        detail.cited_by.len() as u64,
        "the node's memories count and its cited_by list share one predicate"
    );

    let untouched =
        api::graph_nodes::detail(&mut store.ctx(), "file:demo:src/c.rs", None).expect("detail c");
    assert!(untouched.cited_by.is_empty());
    assert_eq!(untouched.node.memories, 0);
}

#[test]
fn neighbors_lists_the_real_imports_counterpart() {
    let mut store = indexed_store();
    let rows = api::graph_nodes::neighbors(
        &mut store.ctx(),
        "file:demo:src/a.rs",
        None,
        api::graph_nodes::NeighborsRequest { min_weight: None },
    )
    .expect("neighbors");

    let imports: Vec<_> = rows.iter().filter(|r| r.rel == "imports").collect();
    assert_eq!(imports.len(), 1, "rows: {rows:?}");
    assert_eq!(imports[0].path, "src/b.rs");
    assert_eq!(imports[0].repo, REPO);
    assert_eq!(imports[0].weight, 1);
}

#[test]
fn neighbors_match_the_context_bundle_for_a_memory_citing_the_file_ac9() {
    let mut store = indexed_store();
    let memory_id = store.save_citing_a();

    let from_bundle = {
        let cfg = Config::defaults();
        let b = bundle::assemble(
            &store.conn,
            &cfg,
            "alpha entry point",
            &[memory_id],
            &WorkingSet::default(),
        )
        .expect("assemble");
        assert!(
            b.code_refs.iter().any(|c| c.path == "src/a.rs"),
            "the bundle must resolve the cited file: {:?}",
            b.code_refs.iter().map(|c| c.id.clone()).collect::<Vec<_>>()
        );
        b.neighbors
    };
    let from_api = api::graph_nodes::neighbors(
        &mut store.ctx(),
        "file:demo:src/a.rs",
        None,
        api::graph_nodes::NeighborsRequest { min_weight: None },
    )
    .expect("neighbors");

    assert!(!from_bundle.is_empty(), "AC-9 needs a non-empty comparison");
    let key = |rows: &[comemory::graph::neighbors::NeighborRow]| {
        rows.iter()
            .map(|r| (r.path.clone(), r.repo.clone(), r.rel.clone(), r.weight))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        key(&from_bundle),
        key(&from_api),
        "the console's neighbors and `comemory context`'s must be the same rows"
    );
}

#[test]
fn neighbors_honor_a_min_weight_floor() {
    let mut store = indexed_store();
    let kept = api::graph_nodes::neighbors(
        &mut store.ctx(),
        "file:demo:src/a.rs",
        None,
        api::graph_nodes::NeighborsRequest {
            min_weight: Some(1),
        },
    )
    .expect("floor 1");
    let max_weight = kept.iter().map(|r| r.weight).max().unwrap_or(0);
    assert!(max_weight > 0, "rows: {kept:?}");

    let filtered = api::graph_nodes::neighbors(
        &mut store.ctx(),
        "file:demo:src/a.rs",
        None,
        api::graph_nodes::NeighborsRequest {
            min_weight: Some(max_weight + 1),
        },
    )
    .expect("high floor");
    assert!(filtered.is_empty(), "rows: {filtered:?}");
}

#[test]
fn snapshot_is_untruncated_on_a_small_graph_and_totals_its_own_edges() {
    let mut store = indexed_store();
    let snap = api::graph_nodes::snapshot(
        &mut store.ctx(),
        api::graph_nodes::SnapshotRequest {
            repo: Some(REPO.to_string()),
            edge_kinds: None,
            min_weight: None,
        },
    )
    .expect("snapshot");

    assert!(!snap.edges.is_empty(), "snapshot: {snap:?}");
    assert!(!snap.truncated);
    assert_eq!(snap.total_edges, snap.edges.len());
    assert!(!snap.nodes.is_empty());
}

#[test]
fn snapshot_edge_kinds_selects_one_relation_and_rejects_an_unknown_one() {
    let mut store = indexed_store();
    let imports_only = api::graph_nodes::snapshot(
        &mut store.ctx(),
        api::graph_nodes::SnapshotRequest {
            repo: Some(REPO.to_string()),
            edge_kinds: Some("imports".to_string()),
            min_weight: None,
        },
    )
    .expect("imports snapshot");
    assert!(!imports_only.edges.is_empty());
    assert!(imports_only.edges.iter().all(|e| e.rel == "imports"));

    let both = api::graph_nodes::snapshot(
        &mut store.ctx(),
        api::graph_nodes::SnapshotRequest {
            repo: Some(REPO.to_string()),
            edge_kinds: Some("imports, co-changed".to_string()),
            min_weight: None,
        },
    )
    .expect("both snapshot");
    assert!(both.total_edges >= imports_only.total_edges);

    let err = api::graph_nodes::snapshot(
        &mut store.ctx(),
        api::graph_nodes::SnapshotRequest {
            repo: Some(REPO.to_string()),
            edge_kinds: Some("calls".to_string()),
            min_weight: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, Error::BadRequest(_)), "got {err:?}");
}
