#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`comemory::domains::code::remote_view`] over a real migrated database:
//! what a machine with no checkout can answer about a repo a peer shared,
//! and what changes when a matching checkout is connected and indexed.
//!
//! Every generation here is activated through the production store path —
//! record, replace the projection, activate — the same three writes the
//! acceptance transaction makes; the transport half is covered in
//! `sync::replica::code_accept`'s tests.

use crate::test_common::git_sample;

use comemory::config::{Config, Paths};
use comemory::domains::code::index_code::{IndexMode, Request};
use comemory::domains::code::{remote_view, repos};
use comemory::store::code_generation::{self, Generation, State};
use comemory::store::remote_code::{Edge, File, Projection, Symbol};
use comemory::store::replica_journal::ReplicaOrigin;
use comemory::store::{connection, remote_code};
use comemory::utilities::context::Ctx;
use tempfile::tempdir;

/// The label both sides file the repository under.
const REPO: &str = "sample";

fn ctx_over(home: &std::path::Path) -> (Paths, Config, rusqlite::Connection) {
    let paths = Paths::new(home);
    paths.ensure_dirs().expect("ensure dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

/// A peer's projection: two files, one symbol, and one import edge between
/// them carrying the blob it was resolved from as its anchor.
fn shared_projection() -> Projection {
    Projection {
        files: vec![
            File {
                path: "src.rs".to_string(),
                blob_oid: "aaaa1111".to_string(),
            },
            File {
                path: "peer_only.rs".to_string(),
                blob_oid: "bbbb2222".to_string(),
            },
        ],
        symbols: vec![Symbol {
            path: "peer_only.rs".to_string(),
            symbol: "peer_entry".to_string(),
            kind: "function".to_string(),
            lang: "rust".to_string(),
            line_start: 1,
            line_end: 4,
        }],
        edges: vec![Edge {
            rel: "imports".to_string(),
            src_path: "peer_only.rs".to_string(),
            dst_path: "src.rs".to_string(),
            weight: 1,
            anchor: Some("bbbb2222".to_string()),
        }],
    }
}

/// Activate one generation for `REPO` exactly as acceptance does.
fn activate(conn: &rusqlite::Connection, generation_id: &str, projection: &Projection) {
    code_generation::record(
        conn,
        &Generation {
            repo: REPO.to_string(),
            generation_id: generation_id.to_string(),
            parent_id: None,
            head: "peer-head-1".to_string(),
            mined_commit: Some("peer-commit-1".to_string()),
            origin: ReplicaOrigin::Sync,
            state: State::Staged,
            file_count: i64::try_from(projection.files.len()).expect("fits"),
            manifest_digest: "d".repeat(64),
        },
        "2026-09-22T10:00:00Z",
    )
    .expect("record");
    remote_code::replace_generation(conn, REPO, generation_id, projection).expect("projection");
    code_generation::activate(conn, REPO, generation_id, "2026-09-22T10:01:00Z").expect("activate");
}

/// Index the real fixture checkout under the same label.
fn index_checkout(
    paths: &Paths,
    cfg: &Config,
    conn: &mut rusqlite::Connection,
    root: &std::path::Path,
) {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    comemory::domains::code::index_code::run(
        &mut ctx,
        Request {
            repo: REPO.into(),
            path: root.to_str().expect("utf8 path").to_string(),
            mode: IndexMode::Incremental,
        },
    )
    .expect("index_code run");
}

#[test]
fn a_machine_with_no_checkout_answers_repos_and_the_graph_but_not_search() {
    let home = tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = ctx_over(home.path());
    activate(
        &conn,
        "1111111111111111aaaaaaaaaaaaaaaa",
        &shared_projection(),
    );

    let shared = remote_view::repos(&conn).expect("repos");
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].repo, REPO);
    assert_eq!(shared[0].head, "peer-head-1");
    assert_eq!(shared[0].file_count, 2);

    let edges = remote_view::graph_edges(&conn, Some(REPO)).expect("graph_edges");
    assert_eq!(edges.len(), 1, "the shared side answers the graph");
    assert_eq!(edges[0].src_id, format!("file:{REPO}:peer_only.rs"));
    assert_eq!(edges[0].dst_id, format!("file:{REPO}:src.rs"));
    assert_eq!(edges[0].rel, "imports");

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let listed = repos::run(&mut ctx, repos::Request { repo: None }).expect("repos::run");
    assert_eq!(listed.repos.len(), 1, "the inventory lists it");
    assert_eq!(listed.repos[0].repo, REPO);
    assert_eq!(listed.repos[0].status, "shared");
    assert_eq!(listed.repos[0].shared_head.as_deref(), Some("peer-head-1"));
    assert_eq!(listed.repos[0].last_head, None, "nothing was indexed here");
    assert_eq!(listed.repos[0].symbols, 0);

    assert!(
        comemory::domains::retrieval::code_search::search_code_hits(
            &cfg,
            &conn,
            "peer_entry",
            None,
            Some(REPO),
            None,
            20,
        )
        .expect("search-code")
        .is_empty(),
        "a shared generation carries no snippet, so source search has nothing to return"
    );
}

#[test]
fn connecting_a_checkout_shows_one_repo_with_both_revisions() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let root = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(home.path());
    activate(
        &conn,
        "1111111111111111aaaaaaaaaaaaaaaa",
        &shared_projection(),
    );

    index_checkout(&paths, &cfg, &mut conn, &root);

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let listed = repos::run(&mut ctx, repos::Request { repo: None }).expect("repos::run");
    assert_eq!(
        listed.repos.len(),
        1,
        "one repository, not one per side: {listed:?}"
    );
    let row = &listed.repos[0];
    assert!(
        row.last_head.is_some(),
        "the local revision is what this machine indexed"
    );
    assert_eq!(
        row.shared_head.as_deref(),
        Some("peer-head-1"),
        "and the shared revision rides the same row"
    );
    assert!(row.symbols > 0, "local symbols are counted");
    assert!(
        !comemory::domains::retrieval::code_search::search_code_hits(
            &cfg,
            &conn,
            "helper",
            None,
            Some(REPO),
            None,
            20,
        )
        .expect("search-code")
        .is_empty(),
        "and local snippets now answer source search"
    );
}

#[test]
fn an_edge_both_sides_know_appears_once_carrying_the_local_weight() {
    let home = tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = ctx_over(home.path());
    // The local index already states this pair, at a weight mining earned.
    comemory::store::edges::insert_weighted(
        &conn,
        comemory::store::edges::EdgeKey {
            src_kind: "file",
            src_id: &format!("file:{REPO}:peer_only.rs"),
            dst_kind: "file",
            dst_id: &format!("file:{REPO}:src.rs"),
            rel: "imports",
        },
        7,
    )
    .expect("local edge");
    activate(
        &conn,
        "1111111111111111aaaaaaaaaaaaaaaa",
        &shared_projection(),
    );

    assert!(
        remote_view::graph_edges(&conn, Some(REPO))
            .expect("graph_edges")
            .is_empty(),
        "the shared side contributes nothing the local index already states"
    );

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let answered = comemory::domains::graph::view::run(
        &mut ctx,
        serde_json::from_value(serde_json::json!({ "repo": REPO })).expect("request"),
    )
    .expect("graph");
    let edges = match answered {
        comemory::domains::graph::view::Response::Full(graph) => graph.edges,
        comemory::domains::graph::view::Response::Paged(page) => page.edges,
    };
    let matching: Vec<_> = edges
        .iter()
        .filter(|e| e.src == format!("file:{REPO}:peer_only.rs"))
        .collect();
    assert_eq!(matching.len(), 1, "one edge, not two: {matching:?}");
    assert_eq!(matching[0].weight, 7, "and it is the local one");
}

#[test]
fn replaying_a_generation_leaves_every_edge_weight_identical() {
    let home = tempdir().expect("tempdir");
    let (_paths, _cfg, conn) = ctx_over(home.path());
    let projection = shared_projection();
    activate(&conn, "1111111111111111aaaaaaaaaaaaaaaa", &projection);
    let first = remote_view::graph_edges(&conn, Some(REPO)).expect("graph_edges");

    // The same generation applied again — the whole projection is replaced,
    // never merged, so nothing accumulates.
    activate(&conn, "1111111111111111aaaaaaaaaaaaaaaa", &projection);

    assert_eq!(
        remote_view::graph_edges(&conn, Some(REPO)).expect("graph_edges"),
        first,
        "a replay is not a second contribution"
    );
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].weight, 1);
}

#[test]
fn an_activation_writes_no_local_edge_and_is_visible_to_the_next_read() {
    let home = tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = ctx_over(home.path());
    let before: i64 = conn
        .query_row("SELECT count(*) FROM edges", [], |r| r.get(0))
        .expect("count edges");
    let fts_before: i64 = conn
        .query_row("SELECT count(*) FROM edge_fts", [], |r| r.get(0))
        .expect("count edge_fts");

    activate(
        &conn,
        "1111111111111111aaaaaaaaaaaaaaaa",
        &shared_projection(),
    );

    assert_eq!(
        conn.query_row("SELECT count(*) FROM edges", [], |r| r.get::<_, i64>(0))
            .expect("count edges"),
        before,
        "a pulled generation writes its own tables, never the local graph"
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM edge_fts", [], |r| r.get::<_, i64>(0))
            .expect("count edge_fts"),
        fts_before,
        "so the triplet index derived from `edges` has nothing to refresh"
    );
    // And the graph reader still answers with the new edge immediately: the
    // union is computed in SQL, so there is no materialized index between an
    // activation and the next read that could go stale.
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let answered = comemory::domains::graph::view::run(
        &mut ctx,
        serde_json::from_value(serde_json::json!({ "repo": REPO })).expect("request"),
    )
    .expect("graph");
    let edges = match answered {
        comemory::domains::graph::view::Response::Full(graph) => graph.edges,
        comemory::domains::graph::view::Response::Paged(page) => page.edges,
    };
    assert_eq!(edges.len(), 1, "{edges:?}");
    assert_eq!(edges[0].rel, "imports");
}

#[test]
fn a_shared_edge_keeps_the_revision_it_was_derived_at() {
    let home = tempdir().expect("tempdir");
    let (_paths, _cfg, conn) = ctx_over(home.path());
    let projection = shared_projection();
    activate(&conn, "1111111111111111aaaaaaaaaaaaaaaa", &projection);

    let stored = remote_code::projection(&conn, REPO, "1111111111111111aaaaaaaaaaaaaaaa")
        .expect("projection");

    assert_eq!(
        stored.edges, projection.edges,
        "an import edge arrives with the blob it was resolved from"
    );
    assert_eq!(
        stored.edges[0].anchor.as_deref(),
        Some("bbbb2222"),
        "the anchor says which revision the edge describes"
    );
}
