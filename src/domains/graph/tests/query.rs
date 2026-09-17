// A fixture that cannot create its git repo or open its database has no
// assertion left to make, so `expect` is the right failure here and its
// message names the step. Narrowed to that one lint: nothing else is
// suppressed in this file.
#![allow(clippy::expect_used)]
//! Tests for `comemory::domains::graph::query` against a REAL indexed git
//! repo, covering the two things this module owns after #170 moved it out of
//! delivery.
//!
//! First, [`Rel`] is the crate's ONE relation vocabulary: the `ValueEnum`
//! string table `comemory graph --rel` parses, `GET /api/v1/graph`'s `rel`
//! field parses, and `GET /api/v1/graph/snapshot`'s `edge_kinds` maps onto.
//! These pin the three accepted spellings and the edge sets each selects, so
//! a second vocabulary cannot appear in an adapter without failing here.
//!
//! Second, [`build_code_graph`] returns every node in scope while
//! [`build_graph_page`] derives its nodes from the windowed edges alone —
//! which is exactly why an isolated file appears in the full graph and not in
//! a page.

use crate::test_common::{git_commit, git_repo};

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use comemory::config::{Config, Paths};
use comemory::domains::graph::query::{Rel, build_code_graph, build_graph_page};
use comemory::store::Connection;
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use tempfile::TempDir;

/// Repo label every test in this file indexes under.
const REPO: &str = "demo";

/// Build `<root>/query-repo`: `src/a.rs` declares `mod b;` so one `imports`
/// edge is mined, and `src/lonely.rs` is indexed but referenced by nothing,
/// so it is an isolated node.
fn build_repo(root: &Path) -> PathBuf {
    let repo = root.join("query-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[
            (
                "src/a.rs",
                "mod b;\n\npub fn alpha() {\n    b::beta();\n}\n",
            ),
            ("src/b.rs", "pub fn beta() {}\n"),
            ("src/lonely.rs", "pub fn lonely() {}\n"),
        ],
        "seed a + b + lonely",
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

/// The distinct `rel` values the selected relation actually returns.
fn rel_kinds(conn: &Connection, rel: Rel) -> BTreeSet<String> {
    build_code_graph(conn, Some(REPO), rel, 1)
        .expect("graph")
        .edges
        .into_iter()
        .map(|e| e.rel)
        .collect()
}

#[test]
fn the_rel_vocabulary_accepts_exactly_the_cli_spellings() {
    assert!(matches!(Rel::from_str("all", true), Ok(Rel::All)));
    assert!(matches!(Rel::from_str("imports", true), Ok(Rel::Imports)));
    assert!(matches!(
        Rel::from_str("co-changed", true),
        Ok(Rel::CoChanged)
    ));
    assert!(
        Rel::from_str("co_changed", true).is_err(),
        "the underscore spelling is the stored edge kind, not a --rel value"
    );
    assert!(Rel::from_str("bogus", true).is_err());
}

#[test]
fn each_rel_selects_only_its_own_edge_kinds() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let conn = index_into(home.path(), &repo_root);

    let imports = rel_kinds(&conn, Rel::Imports);
    assert!(
        imports.contains("imports"),
        "the fixture mints an imports edge"
    );
    assert!(
        !imports.contains("co_changed"),
        "Rel::Imports must not leak co-change edges: {imports:?}"
    );

    let cochanged = rel_kinds(&conn, Rel::CoChanged);
    assert!(
        !cochanged.contains("imports"),
        "Rel::CoChanged must not leak import edges: {cochanged:?}"
    );

    let all = rel_kinds(&conn, Rel::All);
    assert!(
        all.is_superset(&imports) && all.is_superset(&cochanged),
        "Rel::All is the union of both kinds"
    );
}

#[test]
fn min_weight_never_drops_imports_which_always_carry_weight_one() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let conn = index_into(home.path(), &repo_root);

    let kept = build_code_graph(&conn, Some(REPO), Rel::Imports, 1)
        .expect("weight 1")
        .edges
        .len();
    let floored = build_code_graph(&conn, Some(REPO), Rel::Imports, 99)
        .expect("weight 99")
        .edges
        .len();

    assert_eq!(
        kept, floored,
        "the weight floor applies to co_changed only; imports survive any floor"
    );
}

#[test]
fn the_full_graph_keeps_isolated_nodes_and_a_page_does_not() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let conn = index_into(home.path(), &repo_root);

    let lonely = format!("file:{REPO}:src/lonely.rs");

    let whole = build_code_graph(&conn, Some(REPO), Rel::All, 1).expect("whole graph");
    assert!(
        whole.nodes.iter().any(|n| n.id == lonely),
        "build_code_graph fetches every node in scope, edges or not"
    );

    let page = build_graph_page(&conn, Some(REPO), Rel::Imports, 1, 50, 0).expect("page");
    assert!(
        !page.nodes.iter().any(|n| n.id == lonely),
        "a page's nodes are derived from its windowed edges' endpoints only"
    );
}

#[test]
fn nodes_come_back_in_a_stable_id_ascending_order() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let conn = index_into(home.path(), &repo_root);

    let ids: Vec<String> = build_code_graph(&conn, Some(REPO), Rel::All, 1)
        .expect("whole graph")
        .nodes
        .into_iter()
        .map(|n| n.id)
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();

    assert_eq!(
        ids, sorted,
        "node order must be deterministic and ascending"
    );
}

#[test]
fn an_unknown_repo_scope_yields_an_empty_graph_rather_than_an_error() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_repo(workspace.path());
    let conn = index_into(home.path(), &repo_root);

    let graph = build_code_graph(&conn, Some("no-such-repo"), Rel::All, 1).expect("empty graph");

    assert!(graph.nodes.is_empty());
    assert!(graph.edges.is_empty());
}
