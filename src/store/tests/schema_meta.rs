#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/schema_meta.rs`.

use comemory::store::connection;
use comemory::store::migrate;
use comemory::store::schema_meta::{
    get, memory_vector_model, set_memory_vector_model, upsert, version,
};
use tempfile::tempdir;

#[test]
fn version_reads_the_applied_schema_version() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    assert_eq!(version(&conn).expect("version"), migrate::CURRENT_VERSION);
}

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
fn memory_vector_model_defaults_empty_then_reads_back_what_was_set() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    assert_eq!(
        memory_vector_model(&conn).expect("model on a fresh db"),
        String::new(),
        "migration 0016 stamps an empty default"
    );

    set_memory_vector_model(&conn, "ollama:nomic-embed-text").expect("set");
    assert_eq!(
        memory_vector_model(&conn).expect("model after set"),
        "ollama:nomic-embed-text"
    );
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
