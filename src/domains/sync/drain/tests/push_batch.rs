#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Preparing an outbox row for the wire: the canonical repository and the
//! session key are persisted before the first send, so every retry carries
//! what the first attempt carried.

use std::collections::BTreeMap;

use crate::domains::sync::drain::push_batch;
use crate::domains::sync::drain::test_support::LiveEngine;
use crate::domains::sync::replica::test_support::{BODY, Home};
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::store::replica_outbox::{self, Scope};
use crate::store::sync_exchange::ExchangeKey;
use crate::store::sync_policy_snapshot::{self, PolicySnapshot};

fn setup(engine: &LiveEngine, home: &Home) -> (ExchangeKey, RepositoryPolicy, ()) {
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
    (key, policy, ())
}

#[test]
fn prepare_persists_the_canonical_repository_and_the_key_before_the_first_send() {
    let engine = LiveEngine::start();
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    let (key, policy, ()) = setup(&engine, &home);
    let row = replica_outbox::read(&home.conn, Scope::Eligible, 1)
        .expect("row")
        .remove(0);

    let prepared = push_batch::prepare(&home.conn, &key, &policy, row, "t")
        .expect("prepare")
        .expect("payload present");

    assert_eq!(
        prepared.operation.repository.as_deref(),
        Some("falconiere/comemory")
    );
    let stored = replica_outbox::read(&home.conn, Scope::All, 1)
        .expect("row")
        .remove(0);
    assert_eq!(
        stored.wire_repository.as_deref(),
        Some("falconiere/comemory")
    );
    assert_eq!(stored.workspace_id.as_deref(), Some("ws"));
}

/// Staged parts are cut by BYTES, never inside a character: this module's
/// own source (em dashes throughout) crosses whole under every budget.
#[test]
fn staged_parts_stay_within_the_byte_budget_and_never_split_a_character() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/domains/sync/drain/push.rs"
    ))
    .expect("read a real source file");
    assert!(
        text.len() > text.chars().count(),
        "the text holds multi-byte characters"
    );
    for budget in [4, 5, 7, 64, 1024] {
        let parts = super::parts(&text, budget);
        assert!(
            parts.iter().all(|p| p.len() <= budget && !p.is_empty()),
            "budget {budget}"
        );
        assert_eq!(
            parts.concat(),
            text,
            "budget {budget}: the parts are the text"
        );
    }
}
