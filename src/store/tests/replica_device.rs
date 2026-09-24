#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The device id a migrated database carries: minted once, distinct per
//! database, and stable across reopening.

use crate::store::{connection, replica_device};

#[test]
fn a_migrated_database_has_one_stable_device_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let first = {
        let conn = connection::open(&path).expect("open");
        replica_device::id(&conn).expect("device")
    };
    assert_eq!(first.len(), 32);
    assert!(
        first
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
    let conn = connection::open(&path).expect("reopen");
    assert_eq!(
        replica_device::id(&conn).expect("device"),
        first,
        "reopening re-runs no post-pass"
    );
}

#[test]
fn two_databases_never_share_a_device_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let one = connection::open(dir.path().join("one.db")).expect("open one");
    let two = connection::open(dir.path().join("two.db")).expect("open two");
    assert_ne!(
        replica_device::id(&one).expect("one"),
        replica_device::id(&two).expect("two")
    );
}

#[test]
fn a_missing_device_row_is_a_named_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    conn.execute("DELETE FROM replica_device", [])
        .expect("drop the row");
    let error = replica_device::id(&conn).expect_err("no row");
    assert!(error.to_string().contains("replica_device"), "{error}");
}
