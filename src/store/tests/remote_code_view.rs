#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`comemory::store::remote_code_view`] against a real migrated database:
//! which repos a peer shared, and the one definition of what the shared side
//! contributes to the graph — read directly here, and read again through the
//! paginated window in `code_graph_edges`, which must agree.

use comemory::store::code_generation::{self, Generation, State};
use comemory::store::code_graph_edges::{self, EdgeQuery};
use comemory::store::edges::{self, EdgeKey};
use comemory::store::remote_code::{Edge, File, Projection};
use comemory::store::replica_journal::ReplicaOrigin;
use comemory::store::{connection, migrate, remote_code, remote_code_view};
use rusqlite::Connection;
use tempfile::TempDir;

const REPO: &str = "Falconiere/comemory";

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

/// A projection with one file pair joined by an import edge.
fn projection(weight: i64) -> Projection {
    Projection {
        files: vec![File {
            path: "src/lib.rs".to_string(),
            blob_oid: "aaaa1111".to_string(),
        }],
        symbols: Vec::new(),
        edges: vec![Edge {
            rel: "imports".to_string(),
            src_path: "src/lib.rs".to_string(),
            dst_path: "src/store.rs".to_string(),
            weight,
            anchor: Some("aaaa1111".to_string()),
        }],
    }
}

/// Record and activate one generation for `REPO`, extending `parent`.
fn activate_on(
    conn: &Connection,
    generation_id: &str,
    parent: Option<&str>,
    origin: ReplicaOrigin,
    p: &Projection,
) {
    code_generation::record(
        conn,
        &Generation {
            repo: REPO.to_string(),
            generation_id: generation_id.to_string(),
            parent_id: parent.map(str::to_string),
            head: "head-1".to_string(),
            mined_commit: None,
            origin,
            state: State::Staged,
            file_count: i64::try_from(p.files.len()).expect("fits"),
            manifest_digest: "d".repeat(64),
        },
        "2026-09-22T10:00:00Z",
    )
    .expect("record");
    remote_code::replace_generation(conn, REPO, generation_id, p).expect("projection");
    code_generation::activate(conn, REPO, generation_id, "2026-09-22T10:01:00Z").expect("activate");
}

/// The repo's first generation.
fn activate(conn: &Connection, generation_id: &str, origin: ReplicaOrigin, p: &Projection) {
    activate_on(conn, generation_id, None, origin, p);
}

/// The whole unwindowed graph window, for both relation kinds.
fn windowed(conn: &Connection) -> Vec<(String, String, String, i64)> {
    let (rows, total) = code_graph_edges::fetch_page(
        conn,
        &EdgeQuery {
            rels: &["co_changed", "imports"],
            repo: Some(REPO),
            min_weight: 1,
            limit: 0,
            offset: 0,
        },
    )
    .expect("fetch_page");
    assert_eq!(
        total,
        rows.len(),
        "the pre-window total counts the same rows"
    );
    rows.into_iter()
        .map(|r| (r.src_id, r.dst_id, r.rel, r.weight))
        .collect()
}

#[test]
fn only_a_peers_active_generation_is_reported_as_shared() {
    let (_dir, conn) = migrated_db();
    activate(
        &conn,
        "a".repeat(32).as_str(),
        ReplicaOrigin::Sync,
        &projection(3),
    );

    let shared = remote_code_view::shared_repos(&conn).expect("shared_repos");

    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].repo, REPO);
    assert_eq!(shared[0].generation_id, "a".repeat(32));
    assert_eq!(shared[0].head, "head-1");
    assert_eq!(shared[0].file_count, 1);
    assert_eq!(
        shared[0].activated_at.as_deref(),
        Some("2026-09-22T10:01:00Z")
    );
}

#[test]
fn a_locally_built_generation_is_not_reported_as_shared() {
    let (_dir, conn) = migrated_db();
    activate(
        &conn,
        "b".repeat(32).as_str(),
        ReplicaOrigin::Local,
        &projection(3),
    );

    assert!(
        remote_code_view::shared_repos(&conn)
            .expect("shared_repos")
            .is_empty(),
        "what this machine built is the inventory's own subject, not a share"
    );
}

#[test]
fn a_superseded_generation_contributes_nothing() {
    let (_dir, conn) = migrated_db();
    activate(
        &conn,
        "a".repeat(32).as_str(),
        ReplicaOrigin::Sync,
        &projection(3),
    );
    activate_on(
        &conn,
        "c".repeat(32).as_str(),
        Some("a".repeat(32).as_str()),
        ReplicaOrigin::Sync,
        &projection(5),
    );

    let edges = remote_code_view::shared_edges(&conn, Some(REPO)).expect("shared_edges");

    assert_eq!(edges.len(), 1, "one active generation, one contribution");
    assert_eq!(edges[0].weight, 5, "and it is the current one");
    assert_eq!(windowed(&conn).len(), 1, "the window agrees");
}

#[test]
fn the_shared_side_is_keyed_like_a_local_edge() {
    let (_dir, conn) = migrated_db();
    activate(
        &conn,
        "a".repeat(32).as_str(),
        ReplicaOrigin::Sync,
        &projection(3),
    );

    let edges = remote_code_view::shared_edges(&conn, Some(REPO)).expect("shared_edges");

    assert_eq!(edges[0].src_id, format!("file:{REPO}:src/lib.rs"));
    assert_eq!(edges[0].dst_id, format!("file:{REPO}:src/store.rs"));
    assert_eq!(edges[0].rel, "imports");
    assert_eq!(edges[0].weight, 3);
    assert_eq!(
        windowed(&conn),
        vec![(
            format!("file:{REPO}:src/lib.rs"),
            format!("file:{REPO}:src/store.rs"),
            "imports".to_string(),
            3,
        )],
        "a machine with no local edges answers the graph entirely from the share"
    );
}

#[test]
fn a_pair_the_local_index_states_is_never_contributed_twice() {
    let (_dir, conn) = migrated_db();
    edges::insert_weighted(
        &conn,
        EdgeKey {
            src_kind: "file",
            src_id: &format!("file:{REPO}:src/lib.rs"),
            dst_kind: "file",
            dst_id: &format!("file:{REPO}:src/store.rs"),
            rel: "imports",
        },
        9,
    )
    .expect("local edge");
    activate(
        &conn,
        "a".repeat(32).as_str(),
        ReplicaOrigin::Sync,
        &projection(3),
    );

    assert!(
        remote_code_view::shared_edges(&conn, Some(REPO))
            .expect("shared_edges")
            .is_empty(),
        "the local index already states it"
    );
    assert_eq!(
        windowed(&conn),
        vec![(
            format!("file:{REPO}:src/lib.rs"),
            format!("file:{REPO}:src/store.rs"),
            "imports".to_string(),
            9,
        )],
        "so the graph shows one edge, at the local weight"
    );
}

#[test]
fn another_repos_share_is_out_of_scope() {
    let (_dir, conn) = migrated_db();
    activate(
        &conn,
        "a".repeat(32).as_str(),
        ReplicaOrigin::Sync,
        &projection(3),
    );

    assert!(
        remote_code_view::shared_edges(&conn, Some("Falconiere/other"))
            .expect("shared_edges")
            .is_empty()
    );
    assert_eq!(
        remote_code_view::shared_edges(&conn, None)
            .expect("shared_edges")
            .len(),
        1,
        "and an unscoped read still sees it"
    );
}
