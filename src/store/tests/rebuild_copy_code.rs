#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! A staged code generation the preservation copy would ordinarily drop
//! survives when a `pending` outbox operation still names it (#256): it is an
//! owed upload, and dropping it would lose the outgoing payload the next
//! push has to send.

use comemory::store::connection;
use comemory::store::rebuild_copy::copy_preserved_tables_from_old;
use comemory::store::replica_journal::{NewOperation, ReplicaOp, ReplicaOrigin};
use comemory::store::replica_outbox;
use comemory::store::{code_generation, remote_code};
use tempfile::TempDir;

const REPO: &str = "demo";
const GEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// One staged generation with a real projection row.
fn seed_staged_generation(conn: &rusqlite::Connection) {
    code_generation::record(
        conn,
        &code_generation::Generation {
            repo: REPO.to_string(),
            generation_id: GEN.to_string(),
            parent_id: None,
            head: "head-1".to_string(),
            mined_commit: None,
            origin: ReplicaOrigin::Local,
            state: code_generation::State::Staged,
            file_count: 1,
            manifest_digest: "d".repeat(64),
        },
        "2026-09-25T10:00:00Z",
    )
    .expect("record generation");
    remote_code::replace_generation(
        conn,
        REPO,
        GEN,
        &remote_code::Projection {
            files: vec![remote_code::File {
                path: "src/lib.rs".to_string(),
                blob_oid: "aaaa1111".to_string(),
            }],
            symbols: Vec::new(),
            edges: Vec::new(),
        },
    )
    .expect("projection");
}

#[test]
fn a_staged_generation_with_a_pending_push_survives_the_copy() {
    let old_dir = TempDir::new().expect("old tempdir");
    let old_path = old_dir.path().join("comemory.db");
    {
        let old = connection::open(&old_path).expect("old db");
        seed_staged_generation(&old);
        replica_outbox::enqueue(
            &old,
            &NewOperation {
                operation_id: "op-20260925-genpending000000000000000001",
                entity_kind: "code_generation",
                entity_key: REPO,
                op: ReplicaOp::Upsert,
                payload: None,
                schema_version: 1,
                repository: Some(REPO),
                origin: ReplicaOrigin::Local,
                at: "2026-09-25T10:00:00Z",
            },
            None,
        )
        .expect("enqueue owed push");
    }

    let new_dir = TempDir::new().expect("new tempdir");
    let mut conn = connection::open(new_dir.path().join("comemory.db")).expect("new db");
    copy_preserved_tables_from_old(&mut conn, &old_path).expect("copy");

    let generation = code_generation::by_id(&conn, REPO, GEN)
        .expect("by_id")
        .expect("the owed generation survived the copy");
    assert_eq!(generation.state, code_generation::State::Staged);
    assert_eq!(
        remote_code::projection(&conn, REPO, GEN)
            .expect("projection")
            .files
            .len(),
        1,
        "and its projection with it"
    );
}

#[test]
fn a_staged_generation_with_no_pending_push_is_left_behind() {
    let old_dir = TempDir::new().expect("old tempdir");
    let old_path = old_dir.path().join("comemory.db");
    {
        let old = connection::open(&old_path).expect("old db");
        seed_staged_generation(&old);
    }

    let new_dir = TempDir::new().expect("new tempdir");
    let mut conn = connection::open(new_dir.path().join("comemory.db")).expect("new db");
    copy_preserved_tables_from_old(&mut conn, &old_path).expect("copy");

    assert_eq!(
        code_generation::by_id(&conn, REPO, GEN).expect("by_id"),
        None,
        "an upload that never activated and owes nothing publishes nothing"
    );
}
