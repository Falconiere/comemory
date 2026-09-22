#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`comemory::domains::sync::vector_rule`] against a real migrated database
//! — the one verdict both import wires apply to an arriving embedding, and
//! the backlog entry a refusal leaves behind.

use comemory::domains::sync::exchange::SyncVector;
use comemory::domains::sync::vector_rule::{self, Verdict};
use comemory::store::needs_embedding::{self, Reason};
use comemory::store::{connection, migrate, schema_meta, vector};
use rusqlite::Connection;
use tempfile::TempDir;

/// The memory id these cases decide about; the rule never reads the memory
/// itself, only its id.
const MEMORY_ID: &str = "a1b2c3d4";

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

/// A real base64 wire vector of `dims` little-endian `f32`s, unit-length on
/// the first axis so it is a vector this engine could actually store.
fn wire(model: &str, dims: u32) -> SyncVector {
    let mut values = vec![0.0_f32; dims as usize];
    if let Some(first) = values.first_mut() {
        *first = 1.0;
    }
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    SyncVector {
        model: model.to_string(),
        dims,
        f32: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
    }
}

fn local_model(conn: &Connection) -> String {
    schema_meta::memory_vector_model(conn).expect("model")
}

#[test]
fn a_matching_model_at_the_right_dimension_is_stored() {
    let (_dir, conn) = migrated_db();
    let arriving = wire(&local_model(&conn), 1024);

    let verdict = vector_rule::decide(&conn, Some(&arriving)).expect("decide");
    assert!(matches!(verdict, Verdict::Store(_)), "got {verdict:?}");

    vector_rule::apply(&conn, MEMORY_ID, &verdict, "2026-09-22T10:00:00Z").expect("apply");
    assert!(
        vector::memory_embedding_blob(&conn, MEMORY_ID)
            .expect("blob")
            .is_some(),
        "the vector is stored"
    );
    assert_eq!(
        needs_embedding::pending_count(&conn).expect("count"),
        0,
        "and nothing is owed"
    );
}

#[test]
fn a_replay_of_the_same_vector_stores_no_second_row() {
    let (_dir, conn) = migrated_db();
    let arriving = wire(&local_model(&conn), 1024);
    for _ in 0..2 {
        let verdict = vector_rule::decide(&conn, Some(&arriving)).expect("decide");
        vector_rule::apply(&conn, MEMORY_ID, &verdict, "2026-09-22T10:00:00Z").expect("apply");
    }

    let rows: i64 = conn
        .query_row("SELECT count(*) FROM memory_vec", [], |r| r.get(0))
        .expect("count");
    assert_eq!(rows, 1, "a replay replaces rather than duplicates");
}

#[test]
fn a_foreign_model_is_refused_and_recorded_with_what_arrived() {
    let (_dir, conn) = migrated_db();
    let arriving = wire("text-embedding-3-small", 1024);

    let verdict = vector_rule::decide(&conn, Some(&arriving)).expect("decide");
    assert_eq!(
        verdict,
        Verdict::Refuse {
            reason: Reason::Model,
            model: Some("text-embedding-3-small".to_string()),
            dims: Some(1024),
        }
    );

    vector_rule::apply(&conn, MEMORY_ID, &verdict, "2026-09-22T10:00:00Z").expect("apply");
    let pending = needs_embedding::pending(&conn).expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].memory_id, MEMORY_ID);
    assert_eq!(pending[0].reason, Reason::Model);
    assert_eq!(pending[0].model.as_deref(), Some("text-embedding-3-small"));
    assert!(
        vector::memory_embedding_blob(&conn, MEMORY_ID)
            .expect("blob")
            .is_none(),
        "a vector this engine cannot compare against is worse than none"
    );
}

#[test]
fn a_wrong_dimension_is_refused_as_dims_not_as_a_model_mismatch() {
    let (_dir, conn) = migrated_db();
    let arriving = wire(&local_model(&conn), 512);

    let verdict = vector_rule::decide(&conn, Some(&arriving)).expect("decide");
    assert_eq!(
        verdict,
        Verdict::Refuse {
            reason: Reason::Dims,
            model: Some(local_model(&conn)),
            dims: Some(512),
        },
        "the operator action differs: fix the peer's dimension, not the model"
    );
}

#[test]
fn no_vector_at_all_is_refused_as_absent_with_nothing_to_report() {
    let (_dir, conn) = migrated_db();

    let verdict = vector_rule::decide(&conn, None).expect("decide");
    assert_eq!(
        verdict,
        Verdict::Refuse {
            reason: Reason::Absent,
            model: None,
            dims: None,
        }
    );

    vector_rule::apply(&conn, MEMORY_ID, &verdict, "2026-09-22T10:00:00Z").expect("apply");
    let pending = needs_embedding::pending(&conn).expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].reason, Reason::Absent);
    assert_eq!(pending[0].model, None);
    assert_eq!(pending[0].dims, None);
}

#[test]
fn a_later_usable_vector_drains_the_backlog_entry_it_replaces() {
    let (_dir, conn) = migrated_db();
    let refused = vector_rule::decide(&conn, Some(&wire("some-other-model", 1024))).expect("decide");
    vector_rule::apply(&conn, MEMORY_ID, &refused, "2026-09-22T10:00:00Z").expect("apply");
    assert_eq!(needs_embedding::pending_count(&conn).expect("count"), 1);

    let usable = vector_rule::decide(&conn, Some(&wire(&local_model(&conn), 1024))).expect("decide");
    vector_rule::apply(&conn, MEMORY_ID, &usable, "2026-09-22T11:00:00Z").expect("apply");

    assert_eq!(
        needs_embedding::pending_count(&conn).expect("count"),
        0,
        "the backlog entry is drained when the vector finally arrives"
    );
    assert!(
        vector::memory_embedding_blob(&conn, MEMORY_ID)
            .expect("blob")
            .is_some()
    );
}

#[test]
fn a_body_that_is_not_base64_is_a_protocol_error_not_a_refusal() {
    let (_dir, conn) = migrated_db();
    let arriving = SyncVector {
        model: local_model(&conn),
        dims: 1024,
        f32: "not base64 at all!!".to_string(),
    };

    let err = vector_rule::decide(&conn, Some(&arriving)).expect_err("malformed body");
    assert!(
        matches!(err, comemory::errors::Error::BadRequest(_)),
        "the peer sent something that is not a vector: {err:?}"
    );
}
