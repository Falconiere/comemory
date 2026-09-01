#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for `comemory::graph::neighbors` against a REAL indexed git repo:
//! two Rust files where `src/a.rs` declares `mod b;`, committed together,
//! then walked by `api::index_code::run` so the `imports` (and `co_changed`)
//! edges under test are the ones production mining actually writes.
//!
//! Also pins AC-9's shared-query half: the rows this module returns for a
//! file are byte-for-byte the rows `retrieval::bundle::assemble` puts in
//! `comemory context`'s `neighbors` for a memory citing that file — both
//! call [`file_neighbors`], so the two surfaces cannot drift.

use crate::test_common::{git_commit, git_repo};

use std::path::{Path, PathBuf};

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::graph::neighbors::{DEFAULT_MIN_WEIGHT, file_neighbors};
use comemory::store::connection;
use tempfile::TempDir;

/// Repo label every test in this file indexes under.
const REPO: &str = "demo";

/// Build `<root>/import-repo`: `src/a.rs` declares `mod b;` (so the import
/// resolver mints `file:demo:src/a.rs --imports-> file:demo:src/b.rs`) and
/// both files carry a real function, so both land in `code_symbols`.
pub fn build_import_repo(root: &Path) -> PathBuf {
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
        ],
        "seed a + b",
    );
    repo
}

/// Index `repo_root` into a fresh data-dir at `home`, returning the open
/// connection the assertions read through.
pub fn index_into(home: &Path, repo_root: &Path) -> rusqlite::Connection {
    let paths = Paths::new(home);
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
    conn
}

#[test]
fn one_hop_lists_the_real_imports_counterpart() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_import_repo(workspace.path());
    let conn = index_into(home.path(), &repo_root);

    let rows =
        file_neighbors(&conn, &[(REPO, "src/a.rs")], DEFAULT_MIN_WEIGHT).expect("file_neighbors");
    let imports: Vec<_> = rows.iter().filter(|r| r.rel == "imports").collect();
    assert_eq!(imports.len(), 1, "rows: {rows:?}");
    assert_eq!(imports[0].path, "src/b.rs");
    assert_eq!(imports[0].repo, REPO);
    assert_eq!(imports[0].weight, 1, "imports edges always carry weight 1");
}

#[test]
fn the_walk_is_undirected_so_the_imported_file_sees_its_importer() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_import_repo(workspace.path());
    let conn = index_into(home.path(), &repo_root);

    let rows =
        file_neighbors(&conn, &[(REPO, "src/b.rs")], DEFAULT_MIN_WEIGHT).expect("file_neighbors");
    assert!(
        rows.iter()
            .any(|r| r.rel == "imports" && r.path == "src/a.rs"),
        "b.rs must see its importer a.rs across the undirected walk: {rows:?}"
    );
}

#[test]
fn min_weight_above_every_stored_weight_filters_the_neighborhood_empty() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_import_repo(workspace.path());
    let conn = index_into(home.path(), &repo_root);

    let kept = file_neighbors(&conn, &[(REPO, "src/a.rs")], 1).expect("min_weight 1");
    assert!(!kept.is_empty(), "the floor of 1 keeps every edge");
    let max_weight = kept.iter().map(|r| r.weight).max().unwrap_or(0);

    let filtered =
        file_neighbors(&conn, &[(REPO, "src/a.rs")], max_weight + 1).expect("high min_weight");
    assert!(
        filtered.is_empty(),
        "a floor above every stored weight drops every neighbor: {filtered:?}"
    );
}

#[test]
fn an_empty_seed_set_yields_no_rows() {
    let home = TempDir::new().expect("home");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let conn = connection::open(paths.db_path()).expect("open db");

    let rows = file_neighbors(&conn, &[], DEFAULT_MIN_WEIGHT).expect("empty seeds");
    assert!(rows.is_empty());
}

#[test]
fn duplicate_seeds_collapse_to_one_row_per_neighbor_and_relation() {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let repo_root = build_import_repo(workspace.path());
    let conn = index_into(home.path(), &repo_root);

    let once = file_neighbors(&conn, &[(REPO, "src/a.rs")], DEFAULT_MIN_WEIGHT).expect("once");
    let thrice = file_neighbors(
        &conn,
        &[(REPO, "src/a.rs"), (REPO, "src/a.rs"), (REPO, "src/a.rs")],
        DEFAULT_MIN_WEIGHT,
    )
    .expect("thrice");
    assert_eq!(once.len(), thrice.len(), "seeds are deduplicated");
}
