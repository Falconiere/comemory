#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `POST /sync/code/import` over a projection taken from a real
//! `index-code` run: one data dir indexes the tree, a second one receives
//! the projection and must answer the graph the same way.

use comemory::api;
use comemory::api::sync::{CodeImportRejection, CodeImportRequest, code_import};
use comemory::config::{Config, Paths};
use comemory::store::{Connection, connection, repo_marker};
use comemory::utilities::context::Ctx;

use crate::test_common::code_sync_fixture as fixture;
use crate::test_common::{git_commit, git_repo};

/// A data dir that indexed the fixture tree, and the tree itself.
struct Source {
    _home: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    conn: Connection,
    tree: std::path::PathBuf,
}

fn source() -> Source {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("data"));
    paths.ensure_dirs().unwrap();
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let tree = fixture::write_ts_repo(home.path());
    fixture::index(&paths, &cfg, &mut conn, &tree);
    Source {
        _home: home,
        paths,
        cfg,
        conn,
        tree,
    }
}

/// An empty data dir standing in for the workspace.
struct Workspace {
    _home: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    conn: Connection,
}

fn workspace() -> Workspace {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let conn = connection::open(paths.db_path()).unwrap();
    Workspace {
        _home: home,
        paths,
        cfg: Config::defaults(),
        conn,
    }
}

impl Workspace {
    fn import(&mut self, req: CodeImportRequest) -> comemory::api::sync::CodeImportResponse {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        code_import::run(&mut ctx, req).expect("import")
    }

    fn snapshot(&mut self) -> api::graph_nodes::Snapshot {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        api::graph_nodes::snapshot(&mut ctx, api::graph_nodes::SnapshotRequest::default())
            .expect("snapshot")
    }

    fn symbols(&self, path: &str) -> Vec<(String, String)> {
        let mut stmt = self
            .conn
            .prepare("SELECT symbol, snippet FROM code_symbols WHERE path = ?1 ORDER BY symbol")
            .unwrap();
        stmt.query_map([path], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn import_edges_from(&self, path: &str) -> Vec<String> {
        let src = format!("file:{}:{path}", fixture::REPO);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT dst_id FROM edges WHERE src_id = ?1 AND rel = 'imports' ORDER BY dst_id",
            )
            .unwrap();
        stmt.query_map([src], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn ranks(&self) -> Vec<(String, f64)> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, rank_score FROM code_symbols ORDER BY path, symbol")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
}

fn full_request(src: &Source) -> CodeImportRequest {
    let cochange = comemory::store::code_sync::co_changed_pairs(&src.conn, fixture::REPO)
        .unwrap()
        .into_iter()
        .map(|(from, to, weight)| comemory::api::sync::CoChangeWire { from, to, weight })
        .collect();
    CodeImportRequest {
        repo: fixture::REPO.into(),
        head: repo_marker::last_head(&src.conn, fixture::REPO).unwrap(),
        mined_commit: repo_marker::last_mined_commit(&src.conn, fixture::REPO).unwrap(),
        files: fixture::projection(&src.conn),
        removed: Vec::new(),
        cochange: Some(cochange),
    }
}

#[test]
fn ac8_ac9_import_answers_the_graph_and_the_inventory_without_source() {
    let src = source();
    let mut ws = workspace();
    let req = full_request(&src);
    assert_eq!(req.files.len(), 3);
    let head = req.head.clone().expect("indexed head");
    let resp = ws.import(req);
    assert_eq!(resp.applied, 3);
    assert!(resp.rejected.is_empty(), "{:?}", resp.rejected);
    assert_eq!(resp.head.as_deref(), Some(head.as_str()));

    let snap = ws.snapshot();
    let mut labels: Vec<&str> = snap.nodes.iter().map(|n| n.label.as_str()).collect();
    labels.sort_unstable();
    assert_eq!(labels, ["src/a.ts", "src/b.ts", "src/c.ts"]);
    let imports = snap.edges.iter().filter(|e| e.rel == "imports").count();
    assert_eq!(imports, 3, "{:?}", snap.edges);
    assert!(snap.nodes.iter().any(|n| n.rank > 0.0));

    let mut ctx = Ctx::borrowed(&ws.paths, &ws.cfg, &mut ws.conn);
    let repos =
        crate::domains::code::repos::run(&mut ctx, crate::domains::code::repos::Request::default())
            .unwrap();
    assert_eq!(repos.repos.len(), 1);
    assert_eq!(repos.repos[0].repo, fixture::REPO);
    assert_eq!(repos.repos[0].files, 3);
    assert_eq!(repos.repos[0].last_head.as_deref(), Some(head.as_str()));
    let stats = api::stats::run(&mut ctx, api::stats::Request::default()).unwrap();
    assert_eq!(stats.repos, 1);
    assert!(stats.code_symbols >= 3);

    // Snippet-free: every stored row is empty, nothing reached `code_fts`,
    // and code search answers no rows rather than an error.
    for path in ["src/a.ts", "src/b.ts", "src/c.ts"] {
        for (_, snippet) in ws.symbols(path) {
            assert_eq!(snippet, "");
        }
    }
    let fts: i64 = ws
        .conn
        .query_row("SELECT COUNT(*) FROM code_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fts, 0);
    let mut ctx = Ctx::borrowed(&ws.paths, &ws.cfg, &mut ws.conn);
    let hits = api::search_code::run(
        &mut ctx,
        api::search_code::Request {
            query: "alpha".into(),
            k: Some(10),
            offset: 0,
            repo: None,
            lang: None,
            vector: None,
        },
        false,
    )
    .expect("search-code on a projection-only store");
    assert!(hits.hits.is_empty(), "{} hits", hits.hits.len());
    assert!(
        !hits.index_empty,
        "rows exist; the query simply matches no snippet"
    );
}

#[test]
fn ac5_reimporting_an_unchanged_blob_writes_nothing() {
    let src = source();
    let mut ws = workspace();
    ws.import(full_request(&src));
    let before: Vec<(i64, String)> = {
        let mut stmt = ws
            .conn
            .prepare("SELECT id, indexed_at FROM code_symbols ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let resp = ws.import(full_request(&src));
    assert_eq!(resp.applied, 3, "unchanged files still count as applied");
    let after: Vec<(i64, String)> = {
        let mut stmt = ws
            .conn
            .prepare("SELECT id, indexed_at FROM code_symbols ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(before, after, "rows were rewritten for an unchanged blob");
}

#[test]
fn ac6_a_changed_blob_leaves_only_the_new_symbols_and_edges() {
    let mut src = source();
    let mut ws = workspace();
    ws.import(full_request(&src));
    assert_eq!(ws.symbols("src/b.ts").len(), 1);
    assert_eq!(ws.import_edges_from("src/b.ts").len(), 1);

    // `b.ts` drops its import and gains a second function.
    git_commit::commit_files(
        &src.tree,
        &[(
            "src/b.ts",
            "export function beta(): number {\n  return 2;\n}\nexport function delta(): number {\n  return 4;\n}\n",
        )],
        "rework b",
    );
    fixture::index(&src.paths, &src.cfg, &mut src.conn, &src.tree);
    let resp = ws.import(full_request(&src));
    assert!(resp.rejected.is_empty());

    let names: Vec<String> = ws.symbols("src/b.ts").into_iter().map(|(s, _)| s).collect();
    assert_eq!(names, ["beta", "delta"]);
    assert!(
        ws.import_edges_from("src/b.ts").is_empty(),
        "stale import edge survived"
    );
    // The untouched files kept theirs.
    assert_eq!(ws.import_edges_from("src/c.ts").len(), 2);
}

#[test]
fn ac7_an_invalid_tail_entry_applies_nothing() {
    let src = source();
    let mut ws = workspace();
    let mut req = full_request(&src);
    req.files.push(comemory::api::sync::CodeFileWire {
        path: "../outside.ts".into(),
        blob_oid: "b".repeat(40),
        symbols: Vec::new(),
        imports: Vec::new(),
    });
    let resp = ws.import(req);
    assert_eq!(resp.applied, 0);
    assert_eq!(
        resp.rejected,
        vec![CodeImportRejection {
            path: "../outside.ts".into(),
            reason: "invalid_path: must be relative with no `..`".into(),
        }]
    );
    assert_eq!(resp.head, None);
    assert!(ws.symbols("src/a.ts").is_empty());
    assert_eq!(
        repo_marker::last_head(&ws.conn, fixture::REPO).unwrap(),
        None
    );
    let _ = git_repo::run_git; // fixture pairing (see code_sync_fixture.rs)
}

#[test]
fn removed_paths_drop_rows_and_every_edge_touching_them() {
    let src = source();
    let mut ws = workspace();
    ws.import(full_request(&src));
    let resp = ws.import(CodeImportRequest {
        repo: fixture::REPO.into(),
        head: None,
        mined_commit: None,
        files: Vec::new(),
        removed: vec!["src/a.ts".into()],
        cochange: None,
    });
    assert_eq!(resp.removed, 1);
    assert!(ws.symbols("src/a.ts").is_empty());
    let touching: i64 = ws
        .conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE src_id = 'file:scratch:src/a.ts' OR dst_id = 'file:scratch:src/a.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(touching, 0);
    let snap = ws.snapshot();
    assert!(snap.nodes.iter().all(|n| n.label != "src/a.ts"));
}

#[test]
fn ac10_rank_after_import_equals_an_explicit_recompute() {
    let src = source();
    let mut ws = workspace();
    ws.import(full_request(&src));
    let imported = ws.ranks();
    assert!(imported.iter().any(|(_, r)| *r > 0.0), "{imported:?}");
    let mut ctx = Ctx::borrowed(&ws.paths, &ws.cfg, &mut ws.conn);
    api::graph_recompute::run(&mut ctx, api::graph_recompute::Request::default()).unwrap();
    assert_eq!(ws.ranks(), imported);
}

#[test]
fn a_blob_oid_that_is_not_a_git_object_id_is_refused() {
    use comemory::api::sync::code_import_rules::validate;
    let mut req = full_request(&source());
    let path = req.files[0].path.clone();
    for bad in ["", "abc", "A".repeat(40).as_str(), "g".repeat(40).as_str()] {
        req.files[0].blob_oid = bad.to_string();
        let rejected = validate(&req, 0);
        assert!(
            rejected
                .iter()
                .any(|r| r.path == path && r.reason.starts_with("invalid_blob_oid")),
            "{bad:?} passed: {rejected:?}"
        );
    }
    for good in ["a".repeat(40), "0".repeat(64)] {
        req.files[0].blob_oid = good;
        assert!(validate(&req, 0).is_empty());
    }
}

#[test]
fn a_dot_segment_is_refused_so_it_cannot_alias_a_plain_path() {
    use comemory::api::sync::code_import_rules::validate;
    let mut req = full_request(&source());
    req.files[0].path = "src/./a.ts".into();
    let rejected = validate(&req, 0);
    assert!(
        rejected
            .iter()
            .any(|r| r.path == "src/./a.ts" && r.reason.starts_with("invalid_path")),
        "{rejected:?}"
    );
}

#[test]
fn a_repo_label_with_a_colon_is_refused_before_it_can_alias_another_repo() {
    let src = source();
    let mut ws = workspace();
    let mut req = full_request(&src);
    req.repo = "scratch:evil".into();
    let resp = ws.import(req);
    assert_eq!(resp.applied, 0);
    assert!(
        resp.rejected
            .iter()
            .any(|r| r.reason == "invalid_repo: label must not contain `:`"),
        "{:?}",
        resp.rejected
    );
    assert!(ws.snapshot().nodes.is_empty());
}

#[test]
fn the_quota_judges_the_post_batch_total_not_the_same_files_twice() {
    use comemory::store::code_sync::symbol_count_excluding;
    let src = source();
    let mut ws = workspace();
    ws.import(full_request(&src));
    let total = symbol_count_excluding(&ws.conn, fixture::REPO, &[]).unwrap();
    assert_eq!(total, 3);
    // Replacing `a.ts` and `b.ts` leaves only `c.ts`'s symbol standing.
    assert_eq!(
        symbol_count_excluding(&ws.conn, fixture::REPO, &["src/a.ts", "src/b.ts"]).unwrap(),
        1
    );
    // A re-import of everything is judged against zero standing rows —
    // never against the 3 it is about to replace plus the 3 it carries.
    let resp = ws.import(full_request(&src));
    assert!(resp.rejected.is_empty(), "{:?}", resp.rejected);
}

#[test]
fn the_quota_refuses_a_batch_that_would_overflow_the_repo() {
    use comemory::api::sync::code_import_rules::{SYMBOL_QUOTA, validate};
    let req = CodeImportRequest {
        repo: "r".into(),
        head: None,
        mined_commit: None,
        files: vec![comemory::api::sync::CodeFileWire {
            path: "a.rs".into(),
            blob_oid: "a".repeat(40),
            symbols: vec![comemory::api::sync::CodeSymbolWire {
                symbol: "f".into(),
                kind: "function".into(),
                lang: "rust".into(),
                line_start: 1,
                line_end: 1,
            }],
            imports: Vec::new(),
        }],
        removed: Vec::new(),
        cochange: None,
    };
    assert!(validate(&req, SYMBOL_QUOTA - 1).is_empty());
    let over = validate(&req, SYMBOL_QUOTA);
    assert!(
        over.iter().any(|r| r.reason.starts_with("code_quota")),
        "{over:?}"
    );
}
