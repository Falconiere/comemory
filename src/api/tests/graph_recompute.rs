#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for `comemory::api::graph_recompute` (console-api spec §5) over a
//! REAL indexed git repo, so the PageRank it re-derives runs against the
//! `imports`/`co_changed` edges `index-code` actually mined.
//!
//! The load-bearing property is idempotence: PageRank here is deterministic,
//! so recomputing an unchanged graph must leave every `code_symbols.rank_score`
//! bit-identical — a drift would mean the recompute reads a different graph
//! than the indexer's own projection did.

use crate::test_common::{git_commit, git_repo};

use std::path::{Path, PathBuf};

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::store::connection;
use tempfile::TempDir;

/// Repo label every test in this file indexes under.
const REPO: &str = "demo";

/// Build `<root>/import-repo`: `src/a.rs` declares `mod b;`, so the import
/// resolver mints a real edge for PageRank to flow along.
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
        ],
        "seed a + b",
    );
    repo
}

/// A fresh data-dir, optionally with [`build_import_repo`] indexed into it.
fn store(indexed: bool) -> (TempDir, TempDir, Paths, Config, rusqlite::Connection) {
    let workspace = TempDir::new().expect("workspace");
    let home = TempDir::new().expect("home");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).expect("open db");
    if indexed {
        let repo_root = build_import_repo(workspace.path());
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
    (workspace, home, paths, cfg, conn)
}

/// Every `(path, symbol, rank_score)` triple for `REPO`, path-ordered.
fn ranks(conn: &rusqlite::Connection) -> Vec<(String, String, f64)> {
    let mut stmt = conn
        .prepare(
            "SELECT path, symbol, rank_score FROM code_symbols \
              WHERE repo = ?1 ORDER BY path, symbol",
        )
        .expect("prepare ranks");
    stmt.query_map([REPO], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query ranks")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect ranks")
}

#[test]
fn recompute_rescores_every_indexed_repo() {
    let (_ws, _home, paths, cfg, mut conn) = store(true);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let resp = api::graph_recompute::run(&mut ctx, api::graph_recompute::Request {})
        .expect("graph_recompute run");

    assert_eq!(resp.repos, vec![REPO.to_string()]);
    assert!(
        resp.symbols_scored > 0,
        "an indexed repo must score rows: {resp:?}"
    );
    assert_eq!(resp.memories_scored, 0, "no memories were saved");
}

#[test]
fn recompute_leaves_an_unchanged_graph_bit_identical() {
    let (_ws, _home, paths, cfg, mut conn) = store(true);
    let before = ranks(&conn);
    assert!(
        before.iter().any(|(_, _, score)| *score > 0.0),
        "index-code must have materialized a real PageRank: {before:?}"
    );

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::graph_recompute::run(&mut ctx, api::graph_recompute::Request {})
            .expect("first recompute");
    }
    let after_one = ranks(&conn);
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::graph_recompute::run(&mut ctx, api::graph_recompute::Request {})
            .expect("second recompute");
    }
    let after_two = ranks(&conn);

    assert_eq!(
        before, after_one,
        "PageRank over an unchanged graph is fixed"
    );
    assert_eq!(after_one, after_two, "and stays fixed on a repeat run");
}

#[test]
fn recompute_counts_live_memories_as_the_memory_side() {
    let (_ws, _home, paths, cfg, mut conn) = store(true);
    for body in [
        "The alpha entry point lives in `demo:src/a.rs`.",
        "Beta is the callee that alpha reaches.",
    ] {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(
            &mut ctx,
            api::save::Request {
                body: body.to_string(),
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
            },
            false,
            None,
        )
        .expect("save");
    }

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let resp = api::graph_recompute::run(&mut ctx, api::graph_recompute::Request {})
        .expect("graph_recompute run");
    assert_eq!(resp.memories_scored, 2, "resp: {resp:?}");
}

#[test]
fn recompute_on_an_empty_store_is_a_no_op() {
    let (_ws, _home, paths, cfg, mut conn) = store(false);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let resp = api::graph_recompute::run(&mut ctx, api::graph_recompute::Request {})
        .expect("graph_recompute run");
    assert!(resp.repos.is_empty(), "resp: {resp:?}");
    assert_eq!(resp.symbols_scored, 0);
    assert_eq!(resp.memories_scored, 0);
}
