#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/code_graph_edges.rs` — the dynamic, paginated
//! file→file `edges` window behind `comemory graph`.

use comemory::store::code_graph_edges::{EdgeQuery, fetch_page};
use comemory::store::connection;
use comemory::store::edges::{self, EdgeKey};

fn seed_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
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
