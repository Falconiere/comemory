#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! One push batch against a REAL engine on a loopback socket: eligible rows go
//! in the order made under their own ids, each answer lands on its row, the
//! entity is bound, a resend is answered `duplicate` with the original
//! position, and an operation too large for a request crosses in parts.

use std::collections::BTreeMap;

use crate::domains::sync::drain::push::{self, Push};
use crate::domains::sync::drain::test_support::LiveEngine;
use crate::domains::sync::drain::transport::Transport;
use crate::domains::sync::replica::contract::Disposition;
use crate::domains::sync::replica::test_support::{BODY, Home};
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::store::replica_binding;
use crate::store::replica_outbox::{self, Scope};
use crate::store::sync_exchange::ExchangeKey;
use crate::store::sync_policy_snapshot::{self, PolicySnapshot};

fn setup(engine: &LiveEngine, home: &Home) -> (ExchangeKey, RepositoryPolicy, Transport) {
    let key = ExchangeKey::new(&engine.api_url, "ws");
    sync_policy_snapshot::save(
        &home.conn,
        &key,
        &PolicySnapshot {
            revision: 1,
            fingerprint: "fp".into(),
            allowlist: vec!["falconiere/comemory".into()],
            mappings: BTreeMap::new(),
            loaded_at: "t".into(),
        },
    )
    .expect("snapshot");
    let policy = RepositoryPolicy::from_snapshot(&home.conn, &key).expect("policy");
    let transport = Transport::new(
        &engine.api_url,
        &engine.token(),
        std::time::Duration::from_secs(10),
    )
    .expect("transport");
    (key, policy, transport)
}

fn hub_ops(engine: &LiveEngine) -> Vec<String> {
    let conn =
        rusqlite::Connection::open(engine.session.home.path().join("comemory.db")).expect("hub");
    let mut statement = conn
        .prepare("SELECT operation_id FROM replica_feed ORDER BY sequence")
        .expect("prepare");
    statement
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect")
}

#[test]
fn eligible_rows_go_in_order_under_their_ids_and_resends_are_duplicates() {
    let engine = LiveEngine::start();
    let mut home = Home::new();
    let first = home.save(BODY, &["sync"]);
    home.save("A second memory, made after the first.", &["sync"]);
    let (key, policy, transport) = setup(&engine, &home);
    let rows = replica_outbox::read(&home.conn, Scope::Eligible, 10).expect("rows");
    let ids: Vec<String> = rows.iter().map(|r| r.operation_id.clone()).collect();
    let push = Push {
        key: &key,
        transport: &transport,
        policy: &policy,
        cursor: None,
        max_bytes: 4 << 20,
        epoch: None,
        resting: &std::collections::BTreeSet::new(),
    };

    let pushed = push::batch(&home.conn, &push, "t").expect("push");

    assert_eq!((pushed.sent, pushed.accepted, pushed.rejected), (2, 2, 0));
    assert_eq!(
        hub_ops(&engine),
        ids,
        "the hub's feed names the ids this device minted, in order"
    );
    assert!(
        replica_outbox::read(&home.conn, Scope::All, 10)
            .expect("pending")
            .is_empty()
    );
    let bound = replica_binding::get(&home.conn, &key, "memory", &first)
        .expect("binding")
        .expect("bound");
    assert_eq!(bound.synced_sequence, Some(1));

    // Resending the same bytes under the same ids is answered from receipts.
    home.conn
        .execute("UPDATE replica_operation SET state = 'pending'", [])
        .expect("pretend the answers were lost");
    let again = push::batch(&home.conn, &push, "t").expect("resend");
    assert_eq!(again.accepted, 2);
    assert_eq!(hub_ops(&engine).len(), 2, "no second position");
    let dispositions: Vec<String> = home
        .conn
        .prepare("SELECT disposition FROM replica_operation ORDER BY created_at, rowid")
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");
    assert_eq!(dispositions, vec![Disposition::Duplicate.as_str(); 2]);
}

#[test]
fn an_operation_over_the_request_budget_crosses_in_parts() {
    let engine = LiveEngine::start();
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    let (key, policy, transport) = setup(&engine, &home);
    let push = Push {
        key: &key,
        transport: &transport,
        policy: &policy,
        cursor: None,
        max_bytes: 256,
        epoch: None,
        resting: &std::collections::BTreeSet::new(),
    };

    let pushed = push::batch(&home.conn, &push, "t").expect("push");

    assert_eq!(pushed.accepted, 1, "{pushed:?}");
    assert_eq!(hub_ops(&engine).len(), 1);
}
