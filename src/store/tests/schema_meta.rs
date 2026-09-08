#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/schema_meta.rs`.

use comemory::store::connection;
use comemory::store::schema_meta::{get, set_memory_vector_model, upsert};
use tempfile::tempdir;

#[test]
fn set_memory_vector_model_inserts_then_updates() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");

    set_memory_vector_model(&conn, "ollama:nomic-embed-text").expect("insert");
    let stored: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'memory_vector_model'",
            [],
            |r| r.get(0),
        )
        .expect("row exists");
    assert_eq!(stored, "ollama:nomic-embed-text");

    set_memory_vector_model(&conn, "ollama:mxbai-embed-large").expect("update");
    let updated: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'memory_vector_model'",
            [],
            |r| r.get(0),
        )
        .expect("row exists");
    assert_eq!(updated, "ollama:mxbai-embed-large");
}

#[test]
fn get_is_none_for_an_absent_key() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    assert_eq!(get(&conn, "no-such-key").expect("get"), None);
}

#[test]
fn upsert_then_get_round_trips_and_overwrites() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    upsert(&conn, "lazy_reindex_head:demo", "abc|1").expect("insert");
    assert_eq!(
        get(&conn, "lazy_reindex_head:demo").expect("get"),
        Some("abc|1".to_string())
    );

    upsert(&conn, "lazy_reindex_head:demo", "def|2").expect("update");
    assert_eq!(
        get(&conn, "lazy_reindex_head:demo").expect("get"),
        Some("def|2".to_string())
    );
}
