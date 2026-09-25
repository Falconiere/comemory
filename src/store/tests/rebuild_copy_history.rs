#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The exchange-client half of the rebuild preservation copy: every table and
//! column #255 added is carried from a real old database into a real new one,
//! through the same `copy_preserved_tables_from_old` entry point a rebuild
//! calls — so a rebuilt client keeps its stamps, holds, bindings, cursor,
//! anchor and replay.

use std::collections::BTreeMap;

use comemory::store::connection;
use comemory::store::rebuild_copy::copy_preserved_tables_from_old;
use comemory::store::replica_binding::{self, Binding};
use comemory::store::replica_cursor::{self, Anchor, Cursor};
use comemory::store::replica_journal::{NewOperation, ReplicaOp, ReplicaOrigin};
use comemory::store::replica_outbox;
use comemory::store::replica_outbox_hold::{self, Change, Hold, Target};
use comemory::store::replica_pull_hold::{self, PullHold};
use comemory::store::replica_replay::{self, ReplayRow};
use comemory::store::sync_exchange::{self, ExchangeKey, ExchangeRow};
use comemory::store::sync_policy_snapshot::{self, PolicySnapshot};
use tempfile::TempDir;

const EPOCH: &str = "0123456789abcdef0123456789abcdef";

fn seed(path: &std::path::Path, key: &ExchangeKey) {
    let mut conn = connection::open(path).expect("old db");
    let mut row = ExchangeRow::fresh(key);
    row.protocol = Some("replica-v1".into());
    row.upgrade_through = Some(90);
    row.replay_kind = Some("rebootstrap".into());
    row.replay_state = Some("applying".into());
    sync_exchange::save(&conn, &row, "t").expect("exchange");
    sync_policy_snapshot::save(
        &conn,
        key,
        &PolicySnapshot {
            revision: 7,
            fingerprint: "fp".into(),
            allowlist: vec!["falconiere/comemory".into()],
            mappings: BTreeMap::new(),
            loaded_at: "t".into(),
        },
    )
    .expect("snapshot");
    replica_cursor::save(
        &conn,
        &Cursor {
            api_url: key.api_url.clone(),
            workspace_id: key.workspace_id.clone(),
            stream_epoch: EPOCH.into(),
            applied_sequence: 88,
            anchor: Some(Anchor {
                sequence: 88,
                operation_id: "op-20260924-anchor".into(),
            }),
        },
        "t",
    )
    .expect("cursor");
    replica_pull_hold::record(
        &conn,
        key,
        EPOCH,
        &PullHold {
            from_sequence: 12,
            to_sequence: 12,
            reason: "policy".into(),
            entity_kind: Some("memory".into()),
            entity_key: Some("a1b2c3d4".into()),
            repository: Some("acme/private".into()),
            policy_revision: None,
        },
        "t",
    )
    .expect("hold");
    replica_binding::upsert(
        &conn,
        key,
        &Binding {
            entity_kind: "memory".into(),
            entity_key: "b2c3d4e5".into(),
            synced_digest: Some("d".repeat(64)),
            synced_deleted: false,
            synced_sequence: Some(40),
            synced_epoch: Some(EPOCH.into()),
        },
        "t",
    )
    .expect("binding");
    replica_replay::offer(
        &conn,
        key,
        &ReplayRow {
            entity_kind: "memory".into(),
            entity_key: "c3d4e5f6".into(),
            sequence: 60,
            entry_json: "{}".into(),
        },
    )
    .expect("replay");
    let tx = conn.transaction().expect("tx");
    replica_outbox::enqueue(
        &tx,
        &NewOperation {
            operation_id: "op-20260924-held",
            entity_kind: "memory",
            entity_key: "d4e5f6a7",
            op: ReplicaOp::Tombstone,
            payload: None,
            schema_version: 1,
            repository: Some("comemory"),
            origin: ReplicaOrigin::Local,
            at: "t",
        },
        None,
    )
    .expect("enqueue");
    tx.commit().expect("commit");
    let held = Target::Operation("op-20260924-held");
    replica_outbox_hold::update(&conn, held, Change::Stamp(key), "t").expect("stamp");
    replica_outbox_hold::update(&conn, held, Change::Hold(Some((Hold::Secret, "rule"))), "t")
        .expect("hold");
    replica_outbox_hold::update(
        &conn,
        held,
        Change::Wire {
            repository: Some("falconiere/comemory"),
            observed_sequence: None,
        },
        "t",
    )
    .expect("wire");
}

#[test]
fn exchange_state_survives_the_preservation_copy() {
    let old_dir = TempDir::new().expect("old tempdir");
    let old_path = old_dir.path().join("comemory.db");
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    seed(&old_path, &key);

    let new_dir = TempDir::new().expect("new tempdir");
    let mut conn = connection::open(new_dir.path().join("comemory.db")).expect("new db");
    copy_preserved_tables_from_old(&mut conn, &old_path).expect("copy");

    let exchange = sync_exchange::load(&conn, &key)
        .expect("load")
        .expect("exchange kept");
    assert_eq!(exchange.upgrade_through, Some(90));
    assert_eq!(exchange.replay_state.as_deref(), Some("applying"));
    assert_eq!(
        sync_policy_snapshot::load(&conn, &key)
            .expect("snapshot")
            .map(|s| s.revision),
        Some(7)
    );
    let cursor = replica_cursor::load(&conn, &key.api_url, &key.workspace_id)
        .expect("cursor")
        .expect("cursor kept");
    assert_eq!(cursor.applied_sequence, 88);
    assert_eq!(
        cursor.anchor.map(|a| a.operation_id),
        Some("op-20260924-anchor".to_string())
    );
    assert_eq!(
        replica_pull_hold::list(&conn, &key, EPOCH)
            .expect("holds")
            .len(),
        1
    );
    assert_eq!(
        replica_binding::all(&conn, &key).expect("bindings").len(),
        1
    );
    assert_eq!(
        replica_replay::next(&conn, &key, 10).expect("replay").len(),
        1
    );
    let held = replica_outbox::pending(&conn, 10).expect("pending");
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].hold_reason.as_deref(), Some("secret"));
    assert_eq!(held[0].workspace_id.as_deref(), Some("ws_a"));
    assert_eq!(
        held[0].wire_repository.as_deref(),
        Some("falconiere/comemory")
    );
}
