#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/code_graph_edges.rs` — the dynamic, paginated
//! file→file `edges` window behind `comemory graph`.

use comemory::store::code_generation::{self, Generation, State};
use comemory::store::code_graph_edges::{EdgeQuery, fetch_page};
use comemory::store::edges::EdgeKey;
use comemory::store::remote_code::{self, Edge, File, Projection};
use comemory::store::replica_journal::ReplicaOrigin;
use comemory::store::sync_exchange::ExchangeKey;
use comemory::store::sync_policy_snapshot::{self, PolicySnapshot};
use comemory::store::{connection, edges, migrate, repository_approval};

fn seed_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

/// A migrated database — [`seed_db`] plus every table a pulled generation and
/// a loaded policy need.
fn migrated_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

/// Record and activate one peer-shared generation for `repo`, carrying one
/// import edge between two files.
fn activate_shared(conn: &rusqlite::Connection, repo: &str) {
    let projection = Projection {
        files: vec![File {
            path: "src/lib.rs".to_string(),
            blob_oid: "aaaa1111".to_string(),
        }],
        symbols: Vec::new(),
        edges: vec![Edge {
            rel: "imports".to_string(),
            src_path: "src/lib.rs".to_string(),
            dst_path: "src/store.rs".to_string(),
            weight: 3,
            anchor: Some("aaaa1111".to_string()),
        }],
    };
    code_generation::record(
        conn,
        &Generation {
            repo: repo.to_string(),
            generation_id: "a".repeat(32),
            parent_id: None,
            head: "head-1".to_string(),
            mined_commit: None,
            origin: ReplicaOrigin::Sync,
            state: State::Staged,
            file_count: 1,
            manifest_digest: "d".repeat(64),
        },
        "2026-09-25T10:00:00Z",
    )
    .expect("record");
    remote_code::replace_generation(conn, repo, "a".repeat(32).as_str(), &projection)
        .expect("projection");
    code_generation::activate(conn, repo, "a".repeat(32).as_str(), "2026-09-25T10:01:00Z")
        .expect("activate");
}

/// Load a policy snapshot approving exactly `repos`.
fn load_policy(conn: &rusqlite::Connection, repos: &[&str]) {
    sync_policy_snapshot::save(
        conn,
        &ExchangeKey::new("http://hub/api", "ws"),
        &PolicySnapshot {
            revision: 1,
            fingerprint: "fp".to_string(),
            allowlist: repos.iter().map(|r| (*r).to_string()).collect(),
            mappings: std::collections::BTreeMap::new(),
            loaded_at: "2026-09-25T10:00:00Z".to_string(),
        },
    )
    .expect("save snapshot");
    let resolved: Vec<(String, String)> = repos
        .iter()
        .map(|r| ((*r).to_string(), (*r).to_string()))
        .collect();
    repository_approval::replace_all(conn, &resolved, "2026-09-25T10:00:00Z").expect("approve");
}

fn seed_edge(conn: &rusqlite::Connection, src: &str, dst: &str, rel: &str, weight: i64) {
    if weight == 1 {
        edges::insert(
            conn,
            EdgeKey {
                src_kind: "file",
                src_id: src,
                dst_kind: "file",
                dst_id: dst,
                rel,
            },
        )
        .expect("insert");
    } else {
        edges::insert_weighted(
            conn,
            EdgeKey {
                src_kind: "file",
                src_id: src,
                dst_kind: "file",
                dst_id: dst,
                rel,
            },
            weight,
        )
        .expect("insert weighted");
    }
}

#[test]
fn min_weight_drops_low_weight_co_changed_but_not_imports() {
    let (_d, conn) = seed_db();
    seed_edge(&conn, "file:r:a.rs", "file:r:b.rs", "co_changed", 1);
    seed_edge(&conn, "file:r:a.rs", "file:r:c.rs", "co_changed", 5);
    seed_edge(&conn, "file:r:a.rs", "file:r:d.rs", "imports", 1);

    let (rows, total) = fetch_page(
        &conn,
        &EdgeQuery {
            rels: &["co_changed", "imports"],
            repo: None,
            min_weight: 3,
            limit: 0,
            offset: 0,
        },
    )
    .expect("fetch");
    assert_eq!(total, 2, "the weight-1 co_changed edge must be dropped");
    let dsts: Vec<&str> = rows.iter().map(|r| r.dst_id.as_str()).collect();
    assert_eq!(dsts, vec!["file:r:c.rs", "file:r:d.rs"]);
}

#[test]
fn repo_scope_excludes_other_repos() {
    let (_d, conn) = seed_db();
    seed_edge(&conn, "file:r1:a.rs", "file:r1:b.rs", "imports", 1);
    seed_edge(&conn, "file:r2:a.rs", "file:r2:b.rs", "imports", 1);

    let (rows, total) = fetch_page(
        &conn,
        &EdgeQuery {
            rels: &["imports"],
            repo: Some("r1"),
            min_weight: 1,
            limit: 0,
            offset: 0,
        },
    )
    .expect("fetch");
    assert_eq!(total, 1);
    assert_eq!(rows[0].src_id, "file:r1:a.rs");
}

#[test]
fn window_orders_weight_desc_then_paginates() {
    let (_d, conn) = seed_db();
    seed_edge(&conn, "file:r:a.rs", "file:r:b.rs", "co_changed", 2);
    seed_edge(&conn, "file:r:a.rs", "file:r:c.rs", "co_changed", 9);
    seed_edge(&conn, "file:r:a.rs", "file:r:d.rs", "co_changed", 5);

    let (page1, total) = fetch_page(
        &conn,
        &EdgeQuery {
            rels: &["co_changed"],
            repo: None,
            min_weight: 1,
            limit: 1,
            offset: 0,
        },
    )
    .expect("fetch page 1");
    assert_eq!(total, 3);
    assert_eq!(page1.len(), 1);
    assert_eq!(page1[0].dst_id, "file:r:c.rs", "weight 9 sorts first");

    let (page2, _) = fetch_page(
        &conn,
        &EdgeQuery {
            rels: &["co_changed"],
            repo: None,
            min_weight: 1,
            limit: 1,
            offset: 1,
        },
    )
    .expect("fetch page 2");
    assert_eq!(page2[0].dst_id, "file:r:d.rs", "weight 5 sorts second");
}

#[test]
fn a_revoked_repos_shared_edges_drop_from_the_window_and_return_on_reapproval() {
    const REPO: &str = "Falconiere/comemory";
    let (_dir, conn) = migrated_db();
    activate_shared(&conn, REPO);
    let query = EdgeQuery {
        rels: &["imports"],
        repo: Some(REPO),
        min_weight: 1,
        limit: 0,
        offset: 0,
    };

    // No policy loaded at all: unfiltered, as today.
    let (_, total) = fetch_page(&conn, &query).expect("fetch, no policy");
    assert_eq!(total, 1, "a hub or bare serve sees the shared edge");

    load_policy(&conn, &[]);
    let (rows, total) = fetch_page(&conn, &query).expect("fetch, revoked");
    assert_eq!(total, 0, "the window drops the revoked repo's shared edge");
    assert!(rows.is_empty());

    load_policy(&conn, &[REPO]);
    let (rows, total) = fetch_page(&conn, &query).expect("fetch, reapproved");
    assert_eq!(total, 1, "reapproval shows it again with no reload");
    assert_eq!(rows[0].weight, 3);
}
