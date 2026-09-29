#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `project_transfer_bindings` read and upsert against a real database: the
//! seed's binding reads back column for column, and a later transfer
//! replaces it instead of adding a second row.

use comemory::store::connection;
use comemory::store::project_binding::{BindingRow, find, upsert};

const SEED: &str = include_str!("../../../tests/fixtures/projects/every_table_seed.sql");
const A: &str = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f";

#[test]
fn upsert_replaces_and_find_reads_the_binding() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    conn.execute_batch(SEED).expect("seed");

    let seeded = find(&conn, A).expect("find").expect("the seed binds A");
    assert_eq!(seeded.direction, "imported");
    assert_eq!(seeded.remote, "ws-origin");
    assert_eq!(
        seeded.remapped_from,
        Some(("user".to_string(), "u-before".to_string()))
    );
    assert_eq!(seeded.transferred_at, 1_759_000_010_000);

    let exported = BindingRow {
        project_id: A.to_string(),
        direction: "exported".to_string(),
        remote: "ws-target".to_string(),
        digest: "e".repeat(64),
        remapped_from: None,
        transferred_at: 1_759_000_020_000,
    };
    upsert(&conn, &exported).expect("upsert");
    assert_eq!(find(&conn, A).expect("find"), Some(exported));
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM project_transfer_bindings", [], |r| {
            r.get(0)
        })
        .expect("count");
    assert_eq!(rows, 1, "a project has one binding");
    assert_eq!(find(&conn, "no-such-project").expect("find"), None);
}
