#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Replica verify against a REAL engine: every kind the engine reads is
//! compared bucket by bucket with the key's bindings; a kind that differs is
//! repaired by a kind-scoped replay and compared again.

use crate::domains::sync::AuthFile;
use crate::domains::sync::drain::session::{Legs, Mode};
use crate::domains::sync::drain::test_support::{LiveEngine, client_of};
use crate::domains::sync::drain::{self, verify::ReplicaVerify};
use crate::domains::sync::replica::test_support::BODY;
use crate::domains::sync::verify::{self, Verified};

fn verified(report: Verified) -> ReplicaVerify {
    match report {
        Verified::Replica(report) => report,
        Verified::Legacy(report) => panic!("a replica-v1 key verified as legacy: {report:?}"),
    }
}

#[test]
fn a_matching_key_verifies_clean_and_a_lost_binding_is_repaired() {
    let engine = LiveEngine::start();
    let mut home = client_of(&engine);
    let saved = home.save(BODY, &["sync"]);
    let auth = AuthFile::load(&home.paths).expect("load").expect("auth");
    let (paths, cfg) = (home.paths.clone(), home.cfg.clone());
    let run = (Mode::Manual, Legs::Both);
    drain::drain(&paths, &cfg, &mut home.conn, &auth, run).expect("drain");

    let clean = verified(verify::verify(&paths, &cfg, &mut home.conn, &auth).expect("verify"));
    let kinds: Vec<&str> = clean.kinds.iter().map(|k| k.kind.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "memory",
            "code_generation",
            "document_revision",
            "feedback_event",
            "activity_event"
        ]
    );
    assert!(
        clean
            .kinds
            .iter()
            .all(|k| k.differing_buckets == 0 && !k.repaired),
        "{clean:?}"
    );
    assert_eq!(clean.held_positions, 0);

    // Forget what the upstream holds for the memory: its kind now differs.
    home.conn
        .execute(
            "DELETE FROM replica_binding WHERE entity_key = ?1",
            rusqlite::params![saved],
        )
        .expect("forget");
    let repaired = verified(verify::verify(&paths, &cfg, &mut home.conn, &auth).expect("verify"));
    let memory = repaired
        .kinds
        .iter()
        .find(|k| k.kind == "memory")
        .expect("memory");
    assert_eq!(
        (memory.differing_buckets, memory.repaired),
        (0, true),
        "{repaired:?}"
    );
}
