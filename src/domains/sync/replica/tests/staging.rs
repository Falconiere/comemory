#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Staged revisions: nothing is published until every declared part has
//! arrived and the assembled bytes match the operation.

use crate::domains::sync::replica::contract::{Disposition, PROTOCOL};
use crate::domains::sync::replica::contract_views::{ActivateRequest, StageRequest};
use crate::domains::sync::replica::{changes, manifest, staging};
use crate::store::replica_read;

use crate::domains::sync::replica::test_support as support;

use support::{BODY, Home, upsert};

/// Split `bytes` into two parts at its midpoint, on a character boundary.
fn halves(bytes: &str) -> (String, String) {
    let mut mid = bytes.len() / 2;
    while !bytes.is_char_boundary(mid) {
        mid += 1;
    }
    (bytes[..mid].to_string(), bytes[mid..].to_string())
}

fn stage_part(home: &mut Home, staging_id: &str, index: i64, count: i64, bytes: &str) {
    let mut ctx = home.ctx();
    staging::stage(
        &mut ctx,
        StageRequest {
            protocol: PROTOCOL.to_string(),
            staging_id: staging_id.to_string(),
            part_index: index,
            part_count: count,
            bytes: bytes.to_string(),
        },
    )
    .expect("stage");
}

#[test]
fn a_staged_revision_is_invisible_until_it_is_activated() {
    let mut author = Home::new();
    let id = author.save(BODY, &["sync"]);
    let payload = author.payload(&id);
    let (bytes, _digest) = payload.canonical().expect("canonical");
    let (first, second) = halves(&bytes);

    let mut peer = Home::new();
    stage_part(&mut peer, "stage-1", 0, 2, &first);

    let mut ctx = peer.ctx();
    let page = changes::run(&mut ctx, 0, 100, None, None).expect("changes");
    assert!(
        page.entries.is_empty(),
        "a half-uploaded revision is not history"
    );
    let mut ctx = peer.ctx();
    let report = manifest::run(&mut ctx).expect("manifest");
    assert!(report.entity_kinds.is_empty());

    let operation = upsert("op-staged", &payload);
    let incomplete = ActivateRequest {
        protocol: PROTOCOL.to_string(),
        staging_id: "stage-1".to_string(),
        operation: crate::domains::sync::replica::contract::Operation {
            payload: None,
            ..operation.clone()
        },
        cursor: None,
    };
    let mut ctx = peer.ctx();
    assert!(
        staging::activate(&mut ctx, incomplete).is_err(),
        "a missing part refuses activation"
    );

    stage_part(&mut peer, "stage-1", 1, 2, &second);
    let mut ctx = peer.ctx();
    let result = staging::activate(
        &mut ctx,
        ActivateRequest {
            protocol: PROTOCOL.to_string(),
            staging_id: "stage-1".to_string(),
            operation: crate::domains::sync::replica::contract::Operation {
                payload: None,
                ..operation
            },
            cursor: None,
        },
    )
    .expect("activate");

    assert_eq!(result.disposition, Disposition::Accepted);
    assert_eq!(replica_read::head(&peer.conn).expect("head"), 1);
    let mut ctx = peer.ctx();
    let page = changes::run(&mut ctx, 0, 100, None, None).expect("changes");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].entity_key, id);
}

#[test]
fn an_activated_upload_is_discarded_so_a_retry_cannot_replay_its_parts() {
    let mut author = Home::new();
    let id = author.save(BODY, &["sync"]);
    let payload = author.payload(&id);
    let (bytes, _) = payload.canonical().expect("canonical");

    let mut peer = Home::new();
    stage_part(&mut peer, "stage-1", 0, 1, &bytes);
    let mut ctx = peer.ctx();
    staging::activate(
        &mut ctx,
        ActivateRequest {
            protocol: PROTOCOL.to_string(),
            staging_id: "stage-1".to_string(),
            operation: crate::domains::sync::replica::contract::Operation {
                payload: None,
                ..upsert("op-staged", &payload)
            },
            cursor: None,
        },
    )
    .expect("activate");

    let state = crate::store::replica_staging::state(&peer.conn, "stage-1").expect("state");
    assert_eq!(
        state.received, 0,
        "the parts are gone once they are history"
    );
}

#[test]
fn a_part_outside_its_upload_or_on_another_protocol_is_refused() {
    let mut peer = Home::new();
    let mut ctx = peer.ctx();
    assert!(
        staging::stage(
            &mut ctx,
            StageRequest {
                protocol: PROTOCOL.to_string(),
                staging_id: "stage-1".to_string(),
                part_index: 2,
                part_count: 2,
                bytes: "{}".to_string(),
            },
        )
        .is_err(),
        "part 2 of 2 does not exist"
    );

    let mut ctx = peer.ctx();
    assert!(
        staging::stage(
            &mut ctx,
            StageRequest {
                protocol: "replica-v2".to_string(),
                staging_id: "stage-1".to_string(),
                part_index: 0,
                part_count: 1,
                bytes: "{}".to_string(),
            },
        )
        .is_err()
    );
}

#[test]
fn an_activation_that_still_carries_a_payload_is_refused() {
    let mut author = Home::new();
    let id = author.save(BODY, &["sync"]);
    let payload = author.payload(&id);
    let (bytes, _) = payload.canonical().expect("canonical");

    let mut peer = Home::new();
    stage_part(&mut peer, "stage-1", 0, 1, &bytes);
    let mut ctx = peer.ctx();
    let refused = staging::activate(
        &mut ctx,
        ActivateRequest {
            protocol: PROTOCOL.to_string(),
            staging_id: "stage-1".to_string(),
            operation: upsert("op-staged", &payload),
            cursor: None,
        },
    );
    assert!(
        refused.is_err(),
        "the staged parts are the payload; carrying both is ambiguous"
    );
}
