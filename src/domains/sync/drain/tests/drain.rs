#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The drain loop through its one entry point, against a REAL engine: a run
//! pushes and pulls to completion and reports one `exchange` leg, the inline
//! push only pushes, and a pull stalled on an entry this build cannot read
//! ends the pass `stalled` while local operations still go out.

use std::time::Duration;

use crate::domains::sync::AuthFile;
use crate::domains::sync::drain::report::End;
use crate::domains::sync::drain::session::{Legs, Mode};
use crate::domains::sync::drain::test_support::{LiveEngine, client_of};
use crate::domains::sync::drain::{Drained, drain};
use crate::domains::sync::replica::test_support::{BODY, Home};
use crate::store::replica_journal::{self, NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin};
use crate::utilities::canonical_json::bytes_and_digest;

fn run(home: &mut Home, mode: Mode, legs: Legs) -> Drained {
    let auth = AuthFile::load(&home.paths).expect("load").expect("auth");
    let (paths, cfg) = (home.paths.clone(), home.cfg.clone());
    drain(&paths, &cfg, &mut home.conn, &auth, (mode, legs)).expect("drain")
}

#[test]
fn a_run_drains_both_ways_and_reports_one_exchange_leg() {
    let engine = LiveEngine::start();
    let mut a = client_of(&engine);
    a.save(BODY, &["sync"]);
    a.save("A second memory A owes.", &["sync"]);

    let pushed = run(&mut a, Mode::Manual, Legs::Both);
    assert_eq!(pushed.exchange.protocol.as_deref(), Some("replica-v1"));
    assert_eq!(
        (pushed.exchange.pushed, pushed.exchange.end),
        (2, End::CaughtUp)
    );
    assert!(!pushed.exchange.more && pushed.legacy.is_none() && pushed.error.is_none());

    let mut b = client_of(&engine);
    let pulled = run(&mut b, Mode::Unattended, Legs::Pull);
    assert_eq!((pulled.exchange.pulled, pulled.exchange.pushed), (2, 0));
    assert_eq!(pulled.exchange.end, End::CaughtUp);
}

#[test]
fn the_inline_push_only_pushes() {
    let engine = LiveEngine::start();
    let mut writer = client_of(&engine);
    writer.save(BODY, &["sync"]);
    run(&mut writer, Mode::Manual, Legs::Both);
    let mut a = client_of(&engine);
    a.save("A save whose inline push runs right after it.", &["sync"]);

    let inline = run(&mut a, Mode::Inline(Duration::from_secs(5)), Legs::Push);

    assert_eq!((inline.exchange.pushed, inline.exchange.pulled), (1, 0));
    assert_eq!(
        inline.exchange.passes, 1,
        "the inline push runs exactly one pass"
    );
}

#[test]
fn an_unreadable_entry_stalls_the_pull_while_local_operations_go_out() {
    let engine = LiveEngine::start();
    let mut writer = client_of(&engine);
    writer.save(BODY, &["sync"]);
    run(&mut writer, Mode::Manual, Legs::Both);
    {
        // What a newer engine writes: a kind this build cannot read.
        let hub = crate::store::connection::open(engine.session.home.path().join("comemory.db"))
            .expect("hub");
        let epoch = replica_journal::stream_epoch(&hub).expect("epoch");
        let (bytes, digest) =
            bytes_and_digest(&serde_json::json!({"kind": "future_kind"})).expect("payload");
        let bytes = String::from_utf8(bytes).expect("utf8");
        replica_journal::append(
            &hub,
            &epoch,
            &NewOperation {
                operation_id: "op-20260924-futurekind00000000000000000000002",
                entity_kind: "future_kind",
                entity_key: "fk-2",
                op: ReplicaOp::Upsert,
                payload: Some(PayloadRef {
                    digest: &digest,
                    bytes: &bytes,
                }),
                schema_version: 1,
                repository: Some("falconiere/comemory"),
                origin: ReplicaOrigin::Local,
                at: "2026-09-24T10:00:00Z",
            },
        )
        .expect("append");
    }
    let mut a = client_of(&engine);
    a.save("A local memory that goes out despite the stall.", &["sync"]);

    let stalled = run(&mut a, Mode::Manual, Legs::Both);

    assert_eq!(stalled.exchange.end, End::Stalled);
    assert!(!stalled.exchange.more);
    assert_eq!((stalled.exchange.pulled, stalled.exchange.pushed), (1, 1));
}
