#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/graph/mod.rs`. Indexes a real git fixture repo
//! (import edge) via `comemory index-code`, then calls `api::graph::run`
//! directly against a `Ctx` opened on the same data-dir — proving the
//! full/page switch reuses `cli::graph::{build_code_graph, build_graph_page}`
//! (`cli::graph` itself is byte-compat tested in `tests/cli__graph.rs`; the
//! HTTP route + legacy-handler parity live in `tests/serve__routes__graph.rs`).

use assert_cmd::Command;
use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

fn request() -> api::graph::Request {
    api::graph::Request {
        repo: None,
        rel: None,
        min_weight: None,
        limit: None,
        offset: None,
    }
}

/// Index a two-file repo with one static import (`main.rs` imports
/// `parser.rs`) into `home`'s data dir via the real binary.
fn seeded_home() -> (tempfile::TempDir, tempfile::TempDir) {
    let home = tempfile::tempdir().expect("tempdir");
    let workspace = tempfile::tempdir().expect("workspace");
    let repo = workspace.path().join("import-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[
            ("main.rs", "mod parser;\n\nfn main() {}\n"),
            ("parser.rs", "fn parse() {}\n"),
        ],
        "seed the import pair",
    );
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "demo", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();
    (home, workspace)
}

#[test]
fn absent_limit_and_offset_return_the_full_graph() {
    let (home, _workspace) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let response = api::graph::run(&mut ctx, request()).expect("graph run");
    match response {
        api::graph::Response::Full(graph) => {
            assert!(!graph.edges.is_empty(), "expected the import edge");
            assert!(
                graph
                    .edges
                    .iter()
                    .any(|e| e.rel == "imports" && e.src.ends_with("main.rs")),
                "expected an imports edge from main.rs: {:?}",
                graph.edges
            );
        }
        api::graph::Response::Paged(_) => panic!("absent limit/offset must return the full graph"),
    }
}

#[test]
fn present_limit_returns_a_windowed_page() {
    let (home, _workspace) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let mut req = request();
    req.limit = Some(1);
    let response = api::graph::run(&mut ctx, req).expect("graph run");
    match response {
        api::graph::Response::Paged(page) => {
            assert_eq!(page.limit, 1);
            assert_eq!(page.offset, 0);
            assert!(page.edges.len() <= 1);
        }
        api::graph::Response::Full(_) => panic!("present limit must return a paged response"),
    }
}

#[test]
fn rel_filters_to_the_selected_relation() {
    let (home, _workspace) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    // Unfiltered first, so the filtered assertion below cannot pass
    // vacuously: this proves the fixture really does carry an `imports`
    // edge for `rel=co-changed` to remove.
    let unfiltered = match api::graph::run(&mut ctx, request()).expect("graph run") {
        api::graph::Response::Full(graph) => graph,
        api::graph::Response::Paged(_) => panic!("unreachable: no window requested"),
    };
    assert!(
        unfiltered.edges.iter().any(|e| e.rel == "imports"),
        "fixture must carry an imports edge to filter out: {:?}",
        unfiltered.edges
    );

    let mut req = request();
    req.rel = Some("co-changed".to_string());
    let response = api::graph::run(&mut ctx, req).expect("graph run");
    match response {
        api::graph::Response::Full(graph) => {
            assert!(
                !graph.edges.iter().any(|e| e.rel == "imports"),
                "rel=co-changed must exclude the imports edge: {:?}",
                graph.edges
            );
            assert!(
                graph.edges.iter().all(|e| e.rel == "co_changed"),
                "rel=co-changed must yield only co_changed edges: {:?}",
                graph.edges
            );
        }
        api::graph::Response::Paged(_) => panic!("unreachable: no window requested"),
    }
}

#[test]
fn invalid_rel_is_a_bad_request() {
    let (home, _workspace) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let mut req = request();
    req.rel = Some("not-a-real-relation".to_string());
    let err = api::graph::run(&mut ctx, req).expect_err("invalid rel must be rejected");
    assert!(
        err.to_string().contains("invalid rel"),
        "unexpected error: {err}"
    );
}
