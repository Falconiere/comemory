// A fixture that cannot create its git repo or open its database has no
// assertion left to make, so `expect` is the right failure here and its
// message names the step. Narrowed to that one lint: nothing else is
// suppressed in this file.
#![allow(clippy::expect_used)]
//! Tests for `comemory::domains::graph::code_graph` — the exported graph
//! model — against a REAL indexed git repo, so the values under test are the
//! ones `domains::graph::query` actually produces rather than hand-built
//! literals.
//!
//! The model is plain data apart from one behavior: [`GraphPage::new`]'s
//! `has_more` derivation, which is what `comemory graph --limit/--offset` and
//! `GET /api/v1/graph`'s paged envelope both report. These pin its three
//! boundaries — a window shorter than the total, a window reaching the total
//! exactly, and an offset past the end — plus the `Node`/`Edge` field mapping
//! the JSON contract serializes.

use crate::test_common::{git_commit, git_repo};

use std::path::{Path, PathBuf};

use comemory::config::{Config, Paths};
use comemory::domains::graph::code_graph::GraphPage;
use comemory::domains::graph::query::{Rel, build_code_graph, build_graph_page};
use comemory::store::Connection;
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use tempfile::TempDir;

/// Repo label every test in this file indexes under.
const REPO: &str = "demo";

/// Build `<root>/model-repo`: `src/a.rs` declares both `mod b;` and `mod c;`,
/// so the import resolver mints two `imports` edges out of one source file
/// and every endpoint carries `code_symbols` rows.
fn build_repo(root: &Path) -> PathBuf {
    let repo = root.join("model-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[
            (
                "src/a.rs",
                "mod b;\nmod c;\n\npub fn alpha() {\n    b::beta();\n    c::gamma();\n}\n",
            ),
            ("src/b.rs", "pub fn beta() {}\n"),
            ("src/c.rs", "pub fn gamma() {}\n"),
        ],
        "seed a + b + c",
    );
    repo
}

/// Index `repo_root` into a fresh data-dir at `home`, returning the open
/// connection the assertions read through.
fn index_into(home: &Path, repo_root: &Path) -> Connection {
    let paths = Paths::new(home);
    paths.ensure_dirs().expect("ensure dirs");
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).expect("open db");
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        crate::domains::code::index_code::run(
            &mut ctx,
            crate::domains::code::index_code::Request {
                repo: REPO.to_string(),
                path: repo_root.to_str().expect("utf8 repo path").to_string(),
                mode: crate::domains::code::index_code::IndexMode::Incremental,
            },
        )
        .expect("index_code run");
    }
    conn
}

/// The whole `imports` graph for the fixture repo, and its edge count.
fn whole_graph(conn: &Connection) -> (usize, comemory::domains::graph::code_graph::CodeGraph) {
    let graph = build_code_graph(conn, Some(REPO), Rel::Imports, 1).expect("whole graph");
    (graph.edges.len(), graph)
}

#[test]
fn a_window_shorter_than_the_total_reports_more_edges_ahead() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let conn = index_into(home.path(), &repo_root);

    let (total, _) = whole_graph(&conn);
    assert!(
        total >= 2,
        "the fixture must mint at least two imports edges, got {total}"
    );

    let page = build_graph_page(&conn, Some(REPO), Rel::Imports, 1, 1, 0).expect("first page");
    assert_eq!(page.edges.len(), 1, "one edge per the requested window");
    assert_eq!(page.limit, 1);
    assert_eq!(page.offset, 0);
    assert_eq!(page.total, total, "total counts every matching edge");
    assert!(page.has_more, "0 + 1 < {total} means more edges follow");
}

#[test]
fn a_window_reaching_the_total_reports_no_more_edges() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let conn = index_into(home.path(), &repo_root);

    let (total, _) = whole_graph(&conn);
    let page = build_graph_page(&conn, Some(REPO), Rel::Imports, 1, total, 0).expect("full page");

    assert_eq!(page.edges.len(), total);
    assert!(
        !page.has_more,
        "0 + {total} == {total} must not claim more edges"
    );
}

#[test]
fn an_offset_past_the_end_yields_an_empty_page_without_claiming_more() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let conn = index_into(home.path(), &repo_root);

    let (total, _) = whole_graph(&conn);
    let page = build_graph_page(&conn, Some(REPO), Rel::Imports, 1, 10, total + 5)
        .expect("page past the end");

    assert!(page.edges.is_empty(), "nothing remains past the last edge");
    assert!(page.nodes.is_empty(), "no edges means no endpoints");
    assert_eq!(page.total, total, "total is the pre-window count");
    assert!(
        !page.has_more,
        "an offset beyond the total must not claim more edges"
    );
}

#[test]
fn nodes_and_edges_carry_the_fields_the_json_contract_serializes() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let conn = index_into(home.path(), &repo_root);

    let (_, graph) = whole_graph(&conn);

    let a = graph
        .nodes
        .iter()
        .find(|n| n.id == format!("file:{REPO}:src/a.rs"))
        .expect("src/a.rs is a node");
    assert_eq!(a.label, "src/a.rs", "label is the repo-relative path");
    assert_eq!(a.repo, REPO);
    assert!(a.symbols >= 1, "the fixture file has a top-level fn");
    assert_eq!(a.memories, 0, "no memory cites this file");

    let edge = graph
        .edges
        .iter()
        .find(|e| e.dst == format!("file:{REPO}:src/b.rs"))
        .expect("a.rs imports b.rs");
    assert_eq!(edge.src, format!("file:{REPO}:src/a.rs"));
    assert_eq!(edge.rel, "imports");
    assert_eq!(edge.weight, 1, "imports always carry weight 1");
}

#[test]
fn has_more_saturates_instead_of_wrapping_on_a_huge_offset() {
    // The one branch a real query cannot reach: `offset + edges.len()` must
    // saturate rather than wrap, so a caller-supplied offset near `usize::MAX`
    // can never produce a false `has_more`.
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let conn = index_into(home.path(), &repo_root);

    let (count, graph) = whole_graph(&conn);
    let page = GraphPage::new(graph, 0, usize::MAX, count);

    assert!(
        !page.has_more,
        "a saturating offset must not wrap into a false has_more"
    );
    assert_eq!(page.total, count);
}
