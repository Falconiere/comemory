#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/schema_meta.rs`.

use comemory::store::connection;
use comemory::store::schema_meta::set_memory_vector_model;
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
