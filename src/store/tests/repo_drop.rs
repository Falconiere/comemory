#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/repo_drop.rs`, over a REALLY indexed temp git
//! repo (`crate::domains::code::index_code::run`) with a real memory saved against it: every
//! code-side row for the label goes, the memory and its `references_file`
//! edge stay, and a second repo indexed in the same store is untouched.

use crate::test_common::git_sample;

use comemory::config::{Config, Paths};
use comemory::domains::code::index_code::IndexMode;
use comemory::domains::memories::Kind;
use comemory::store::edges::{self, EdgeKey};
use comemory::store::{connection, repo_drop};
use comemory::utilities::context::Ctx;
use rusqlite::Connection;
use tempfile::TempDir;

fn ctx_over(home: &TempDir) -> (Paths, Config, Connection) {
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

fn index(ctx: &mut Ctx<'_>, repo: &str, path: &std::path::Path) {
    crate::domains::code::index_code::run(
        ctx,
        crate::domains::code::index_code::Request {
            repo: repo.into(),
            path: path.to_str().expect("utf8 path").to_string(),
            mode: IndexMode::Incremental,
        },
    )
    .expect("index_code run");
}

fn count(conn: &Connection, sql: &str, repo: &str) -> i64 {
    conn.query_row(sql, [repo], |r| r.get(0)).expect("count")
}

/// `code_fts` rows still joined to a live `code_symbols` row for `repo`.
fn fts_rows(conn: &Connection, repo: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM code_fts WHERE symbol_id IN \
         (SELECT id FROM code_symbols WHERE repo = ?1)",
        [repo],
        |r| r.get(0),
    )
    .expect("count code_fts")
}

/// File-node edges (`file:<repo>:<path>` on either side) for `repo`.
fn file_edges(conn: &Connection, repo: &str) -> i64 {
    let prefix = format!("file:{repo}:");
    conn.query_row(
        "SELECT COUNT(*) FROM edges \
          WHERE (src_kind = 'file' AND src_id LIKE ?1 || '%') \
             OR (dst_kind = 'file' AND dst_id LIKE ?1 || '%')",
        [&prefix],
        |r| r.get(0),
    )
    .expect("count edges")
}

/// `edge_fts` triplet rows still naming a `file:<repo>:` node.
fn file_edge_triplets(conn: &Connection, repo: &str) -> i64 {
    let prefix = format!("file:{repo}:");
    conn.query_row(
        "SELECT COUNT(*) FROM edge_fts \
          WHERE src_id LIKE ?1 || '%' OR dst_id LIKE ?1 || '%'",
        [&prefix],
        |r| r.get(0),
    )
    .expect("count edge_fts")
}

/// Save one real memory citing the sample repo, through the production
/// `domains::memories::save` core. Its body mentions `sample:src.rs`, so the
/// save derives a `references_file` edge — the memory-side link a repo
/// disconnect must KEEP (spec AC-18).
fn save_sample_memory(ctx: &mut Ctx<'_>) -> String {
    crate::domains::memories::save::run(
        ctx,
        crate::domains::memories::save::Request {
            body: "the sample repo indexes `sample:src.rs` cleanly".to_string(),
            title: None,
            kind: Kind::Note,
            repo: "sample".to_string(),
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
    .expect("save")
    .id
}

/// Every live memory's `(id, rank_score)`, the OTHER derived artifact
/// `graph::derived::refresh_derived_best_effort` maintains beside `edge_fts`.
/// Read as a string so a `NULL` and a `0.0` are distinguishable.
fn rank_scores(conn: &Connection) -> Vec<(String, String)> {
    conn.prepare(
        "SELECT id, COALESCE(CAST(rank_score AS TEXT), 'NULL') FROM memories \
          WHERE deleted_at IS NULL ORDER BY id",
    )
    .expect("prepare")
    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
    .expect("query")
    .collect::<std::result::Result<Vec<_>, _>>()
    .expect("rank_score rows")
}

/// Seed one file→file `imports` edge inside `repo` so the edge purge has a
/// row to prove itself against even for a single-file sample repo.
fn seed_import_edge(conn: &Connection, repo: &str) {
    let src = format!("file:{repo}:src.rs");
    let dst = format!("file:{repo}:other.rs");
    edges::insert(
        conn,
        EdgeKey {
            src_kind: "file",
            src_id: &src,
            dst_kind: "file",
            dst_id: &dst,
            rel: "imports",
        },
    )
    .expect("seed import edge");
}

#[test]
fn drop_repo_removes_every_code_row_and_file_edge_and_keeps_the_memory() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(&home);
    let memory_id = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        index(&mut ctx, "sample", &repo);
        save_sample_memory(&mut ctx)
    };
    seed_import_edge(&conn, "sample");

    let symbols_before = count(
        &conn,
        "SELECT COUNT(*) FROM code_symbols WHERE repo = ?1",
        "sample",
    );
    assert!(symbols_before > 0, "the fixture must really be indexed");
    assert!(fts_rows(&conn, "sample") > 0, "code_fts must be populated");
    assert!(file_edges(&conn, "sample") > 0, "file edges must exist");
    let memory_edges_before: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE src_kind = 'memory' AND src_id = ?1",
            [&memory_id],
            |r| r.get(0),
        )
        .expect("count memory edges");
    assert!(
        memory_edges_before > 0,
        "the backtick reference must have produced a memory->file edge"
    );

    let counts = repo_drop::drop_repo(&mut conn, "sample").expect("drop_repo");

    assert_eq!(
        counts.symbols_removed,
        u64::try_from(symbols_before).unwrap()
    );
    assert!(counts.files_removed > 0, "{counts:?}");
    assert!(counts.edges_removed > 0, "{counts:?}");
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM code_symbols WHERE repo = ?1",
            "sample"
        ),
        0
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM indexed_files WHERE repo = ?1",
            "sample"
        ),
        0
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM repo_marker WHERE repo = ?1",
            "sample"
        ),
        0
    );
    assert_eq!(fts_rows(&conn, "sample"), 0);
    assert_eq!(file_edges(&conn, "sample"), 0);

    let live: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE id = ?1 AND deleted_at IS NULL",
            [&memory_id],
            |r| r.get(0),
        )
        .expect("count memories");
    assert_eq!(live, 1, "memories are retained");
    let memory_edges_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE src_kind = 'memory' AND src_id = ?1",
            [&memory_id],
            |r| r.get(0),
        )
        .expect("count memory edges");
    assert_eq!(
        memory_edges_after, memory_edges_before,
        "memory→code reference edges are kept"
    );
}

#[test]
fn drop_repo_leaves_a_second_repo_in_the_same_store_untouched() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let first = git_sample::build_sample_repo(&workspace.path().join("one"));
    let second = git_sample::build_sample_repo(&workspace.path().join("two"));
    let (paths, cfg, mut conn) = ctx_over(&home);
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        index(&mut ctx, "repo-a", &first);
        index(&mut ctx, "repo-b", &second);
    }
    seed_import_edge(&conn, "repo-a");
    seed_import_edge(&conn, "repo-b");
    let kept_symbols = count(
        &conn,
        "SELECT COUNT(*) FROM code_symbols WHERE repo = ?1",
        "repo-b",
    );
    let kept_edges = file_edges(&conn, "repo-b");

    repo_drop::drop_repo(&mut conn, "repo-a").expect("drop_repo");

    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM code_symbols WHERE repo = ?1",
            "repo-b"
        ),
        kept_symbols
    );
    assert_eq!(file_edges(&conn, "repo-b"), kept_edges);
    assert!(kept_edges > 0);
}

#[test]
fn drop_repo_does_not_refresh_the_derived_artifacts() {
    // The refresh is the CALLER's post-commit step (#177):
    // `domains::code::repo_admin::disconnect` owns it, and this store helper
    // must leave BOTH derived artifacts exactly as it found them.
    // `refresh_derived_best_effort` maintains two — `memories.rank_score` and
    // the `edge_fts` triplet index — so checking only one would leave half the
    // separation unproven.
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let sample = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(&home);
    let memory_id = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        index(&mut ctx, "sample", &sample);
        save_sample_memory(&mut ctx)
    };
    seed_import_edge(&conn, "sample");
    // Materialize both artifacts through the very seam that was moved, so the
    // "before" state is the one a real disconnect would be invalidating.
    crate::domains::graph::derived::refresh_derived_best_effort(&mut conn);
    let triplets_before = file_edge_triplets(&conn, "sample");
    assert!(
        triplets_before > 0,
        "the fixture must really index its file edges"
    );
    // `rank_score` needs a different instrument from `edge_fts`. Dropping a
    // repo deliberately KEEPS every memory-sourced edge (AC-18), so the real
    // score is the same before and after a refresh — asserting it "unchanged"
    // would prove nothing. Stamp a sentinel no PageRank can produce instead:
    // surviving the drop means no rank pass ran at all.
    conn.execute("UPDATE memories SET rank_score = -1.0", [])
        .expect("stamp the sentinel score");
    let sentinel = rank_scores(&conn);
    assert_eq!(sentinel.len(), 1, "one saved memory");
    assert_eq!(sentinel[0].0, memory_id);

    repo_drop::drop_repo(&mut conn, "sample").expect("drop_repo");

    assert_eq!(file_edges(&conn, "sample"), 0, "the edges themselves go");
    assert_eq!(
        file_edge_triplets(&conn, "sample"),
        triplets_before,
        "drop_repo must not refresh the triplet index; its caller does"
    );
    assert_eq!(
        rank_scores(&conn),
        sentinel,
        "drop_repo must not recompute rank_score either; its caller does"
    );

    // And the sentinel really is refresh-sensitive: running the caller's step
    // now overwrites it. Without this the assertion above could pass simply
    // because nothing ever rewrites `rank_score`.
    crate::domains::graph::derived::refresh_derived_best_effort(&mut conn);
    assert_ne!(
        rank_scores(&conn),
        sentinel,
        "the refresh the caller owns must replace the sentinel score"
    );
}

#[test]
fn dropping_a_repo_that_was_never_indexed_is_a_no_op() {
    let home = TempDir::new().expect("home");
    let (_paths, _cfg, mut conn) = ctx_over(&home);

    let counts = repo_drop::drop_repo(&mut conn, "ghost").expect("drop_repo");

    assert_eq!(counts.symbols_removed, 0);
    assert_eq!(counts.files_removed, 0);
    assert_eq!(counts.edges_removed, 0);
}
