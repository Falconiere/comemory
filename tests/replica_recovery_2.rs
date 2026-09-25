#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Client-side seeding and adoption (#256, B-1): a legacy client, never
//! synced, uploads its whole pre-journal history — live memories, trashed
//! ones and the retained verdict/run — to a real hub on its first sync.

#[path = "common/legacy_engine.rs"]
mod legacy_engine;

#[path = "common/replica_support.rs"]
mod replica_support;

#[path = "common/recovery_support.rs"]
mod recovery_support;

#[path = "common/fault_proxy.rs"]
mod fault_proxy;

#[path = "common/exchange_support.rs"]
mod exchange_support;

use std::collections::HashSet;

const REPO: &str = "legacy-corpus";
const CANONICAL: &str = "acme/legacy-corpus";
const MEMORY_COUNT: usize = 30;
const TRASHED_COUNT: usize = 5;
const WORKSPACE: &str = "ws_recovery";

/// Write the credential and the approval a real device login + policy fetch
/// would leave — the #255/#253 precedent for a client that has no live
/// platform to authenticate against in this suite.
fn make_client(data_dir: &std::path::Path, hub: &exchange_support::Hub) {
    let auth = serde_json::json!({
        "version": 2,
        "secret": hub.token(),
        "key_prefix": "cmk_test",
        "api_url": hub.api_url(),
        "organization_id": "org_recovery",
        "organization_slug": "recovery",
        "organization_name": "Recovery",
        "workspace_id": WORKSPACE,
    });
    std::fs::write(
        data_dir.join("auth.json"),
        serde_json::to_vec_pretty(&auth).expect("auth json"),
    )
    .expect("write auth.json");

    // Migrates on first touch: the legacy binary never applied these tables.
    let conn = comemory::store::connection::open(data_dir.join("comemory.db")).expect("migrate");
    let key = comemory::store::sync_exchange::ExchangeKey::new(&hub.api_url(), WORKSPACE);
    comemory::store::sync_policy_snapshot::save(
        &conn,
        &key,
        &comemory::store::sync_policy_snapshot::PolicySnapshot {
            revision: 1,
            fingerprint: format!("rev-1-{CANONICAL}"),
            allowlist: vec![CANONICAL.to_string()],
            mappings: std::collections::BTreeMap::from([(REPO.to_string(), CANONICAL.to_string())]),
            loaded_at: "2026-09-25T10:00:00Z".to_string(),
        },
    )
    .expect("save snapshot");
    comemory::store::repository_approval::replace_all(
        &conn,
        &[(REPO.to_string(), CANONICAL.to_string())],
        "2026-09-25T10:00:00Z",
    )
    .expect("approve");
}

#[test]
fn a_legacy_client_uploads_its_history_once() {
    let hub = exchange_support::Hub::start();

    let client_home = tempfile::tempdir().expect("client home");
    let data_dir = client_home.path().join(".comemory");
    let corpus =
        recovery_support::build_legacy_corpus(&data_dir, REPO, MEMORY_COUNT, TRASHED_COUNT);
    let (live_id, live_body) = corpus.first_live();
    let live_id = live_id.to_string();
    let query = recovery_support::distinctive_query(live_body);
    let query_id = recovery_support::legacy_verdict_and_runs(&data_dir, REPO, &live_id, &query);

    make_client(&data_dir, &hub);

    // The client's own upgrade (migration + seeding) happens on this first
    // real sync, then it uploads what it seeded.
    let (code, _stdout, stderr) = replica_support::cli_raw(&data_dir, &["sync"]);
    assert_eq!(code, 0, "first sync: {stderr}");

    let hub_data = hub.data_dir();
    let expected_live = MEMORY_COUNT - TRASHED_COUNT;
    let hub_entities = recovery_support::feed_entities_with_op(&hub_data);
    let hub_memory: Vec<&(String, String, String)> = hub_entities
        .iter()
        .filter(|(kind, ..)| kind == "memory")
        .collect();
    let upserts: HashSet<&str> = hub_memory
        .iter()
        .filter(|(_, _, op)| op == "upsert")
        .map(|(_, key, _)| key.as_str())
        .collect();
    let tombstones: HashSet<&str> = hub_memory
        .iter()
        .filter(|(_, _, op)| op == "tombstone")
        .map(|(_, key, _)| key.as_str())
        .collect();
    assert_eq!(
        upserts.len(),
        expected_live,
        "hub holds each live memory once"
    );
    assert_eq!(
        tombstones.len(),
        TRASHED_COUNT,
        "hub holds each tombstone once"
    );

    let (verdict, event_id) = recovery_support::feedback_event_row(&data_dir, &live_id, &query_id)
        .expect("the client itself retained the verdict it made");
    assert_eq!(verdict, "used");
    let event_id = event_id.expect("a captured verdict carries a stable replica event id");
    let (hub_verdict, hub_memory_id) =
        recovery_support::feedback_event_by_event_id(&hub_data, &event_id)
            .expect("the retained verdict reached the hub");
    assert_eq!(hub_verdict, "used");
    assert_eq!(
        hub_memory_id, live_id,
        "the hub resolved the same content-derived memory id"
    );
    assert!(
        recovery_support::shared_activity_count(&hub_data) >= 2,
        "the retained runs reached the hub"
    );

    // A second sync on each side changes no count.
    let before_hub_head = recovery_support::feed_entities(&hub_data).len();
    let (code, _stdout, stderr) = replica_support::cli_raw(&data_dir, &["sync"]);
    assert_eq!(code, 0, "second sync: {stderr}");
    assert_eq!(
        recovery_support::feed_entities(&hub_data).len(),
        before_hub_head,
        "a second sync on the client adds nothing to the hub"
    );
}
