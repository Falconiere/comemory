#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! A replaced stream over a real store: the cursor restarts on the new
//! stream, every replica-epoch pull hold is dropped (legacy holds stay),
//! every binding's position is cleared while its digest stays for verify,
//! and a compacting replay through the head begins. Nothing local is
//! deleted.

use crate::domains::sync::drain::rebootstrap::{self, REBOOTSTRAP};
use crate::domains::sync::replica::test_support::Home;
use crate::store::replica_binding::{self, Binding};
use crate::store::replica_cursor::{self, Anchor, Cursor};
use crate::store::replica_pull_hold::{self, LEGACY_EPOCH, PullHold};
use crate::store::replica_replay::{self, ReplayRow};
use crate::store::sync_exchange::{ExchangeKey, ExchangeRow};

fn hold(at: i64, reason: &str) -> PullHold {
    PullHold {
        from_sequence: at,
        to_sequence: at,
        reason: reason.into(),
        entity_kind: Some("memory".into()),
        entity_key: Some(format!("m{at}")),
        repository: None,
        policy_revision: None,
    }
}

#[test]
fn begin_restarts_the_cursor_and_keeps_only_what_verify_needs() {
    let mut home = Home::new();
    let kept = home.save("A memory that stays local through the rebootstrap.", &[]);
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws");
    replica_pull_hold::record(&home.conn, &key, "old-epoch", &hold(5, "policy"), "t").expect("h");
    replica_pull_hold::record(&home.conn, &key, LEGACY_EPOCH, &hold(7, "secret"), "t").expect("h");
    let binding = Binding {
        entity_kind: "memory".into(),
        entity_key: "m1".into(),
        synced_digest: Some("d".repeat(64)),
        synced_deleted: false,
        synced_sequence: Some(9),
        synced_epoch: Some("old-epoch".into()),
    };
    replica_binding::upsert(&home.conn, &key, &binding, "t").expect("bind");
    let scratch = ReplayRow {
        entity_kind: "memory".into(),
        entity_key: "m1".into(),
        sequence: 9,
        entry_json: "{}".into(),
    };
    replica_replay::offer(&home.conn, &key, &scratch).expect("scratch");
    let mut cursor = Cursor {
        api_url: key.api_url.clone(),
        workspace_id: key.workspace_id.clone(),
        stream_epoch: "old-epoch".into(),
        applied_sequence: 12,
        anchor: Some(Anchor {
            sequence: 12,
            operation_id: "op-x".into(),
        }),
    };
    let mut row = ExchangeRow::fresh(&key);

    rebootstrap::begin(&home.conn, &mut row, &mut cursor, "new-epoch", 42, "t").expect("begin");

    let stored = replica_cursor::load(&home.conn, &key.api_url, &key.workspace_id)
        .expect("load")
        .expect("cursor");
    assert_eq!(
        (stored.stream_epoch.as_str(), stored.applied_sequence),
        ("new-epoch", 0)
    );
    assert!(stored.anchor.is_none());
    assert!(
        replica_pull_hold::list(&home.conn, &key, "old-epoch")
            .expect("l")
            .is_empty()
    );
    assert_eq!(
        replica_pull_hold::list(&home.conn, &key, LEGACY_EPOCH)
            .expect("l")
            .len(),
        1
    );
    let cleared = replica_binding::get(&home.conn, &key, "memory", "m1")
        .expect("get")
        .expect("binding kept");
    assert_eq!(
        cleared.synced_sequence, None,
        "positions of the old stream are void"
    );
    assert_eq!(
        cleared.synced_digest, binding.synced_digest,
        "digests stay for verify"
    );
    assert!(
        replica_replay::next(&home.conn, &key, 10)
            .expect("scratch")
            .is_empty()
    );
    assert_eq!(row.replay_kind.as_deref(), Some(REBOOTSTRAP));
    assert_eq!(row.replay_state.as_deref(), Some("scanning"));
    assert_eq!(
        (row.replay_scan_through, row.replay_target),
        (Some(0), Some(42))
    );
    assert!(
        !home.payload(&kept).body.is_empty(),
        "nothing local is deleted"
    );

    rebootstrap::finish(&mut row);
    assert!(row.replay_kind.is_none() && row.replay_state.is_none() && row.replay_target.is_none());
}
