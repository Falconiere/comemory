#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The ordered page a peer pulls: what it carries, where it stops, and what
//! it says about a payload that is no longer there.

use crate::domains::sync::replica::contract_views::PayloadState;
use crate::domains::sync::replica::{accept, changes};
use crate::store::replica_journal;

use crate::domains::sync::replica::test_support as support;

use support::{BODY, Home, envelope, tombstone, upsert};

#[test]
fn a_page_carries_the_bytes_accepted_at_each_position() {
    let mut home = Home::new();
    let id = home.save(BODY, &["sync"]);
    let mut ctx = home.ctx();
    let page = changes::run(&mut ctx, 0, 100, None, None).expect("changes");

    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].entity_key, id);
    assert_eq!(page.entries[0].payload_state, PayloadState::Present);
    assert_eq!(page.head_sequence, 1);
    assert_eq!(page.next_sequence, Some(1));
    let payload = page.entries[0].payload.as_ref().expect("payload");
    assert_eq!(payload["id"], serde_json::json!(id));
    assert_eq!(payload["body"], serde_json::json!(BODY));
}

#[test]
fn a_cursor_at_the_head_is_caught_up_and_one_above_it_is_refused() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    let mut ctx = home.ctx();
    let caught_up = changes::run(&mut ctx, 1, 100, None, None).expect("changes");

    assert!(caught_up.entries.is_empty());
    assert_eq!(caught_up.next_sequence, None);
    assert_eq!(caught_up.head_sequence, 1, "the head is still reported");

    let mut ctx = home.ctx();
    let ahead = changes::run(&mut ctx, 99, 100, None, None);
    assert!(
        ahead.is_err(),
        "a cursor past the head names a position this stream never issued"
    );
}

#[test]
fn a_tombstone_entry_names_no_payload() {
    let mut home = Home::new();
    let id = home.save(BODY, &["sync"]);
    let mut ctx = home.ctx();
    accept::run(&mut ctx, envelope(vec![tombstone("op-1", &id)])).expect("tombstone");

    let mut ctx = home.ctx();
    let page = changes::run(&mut ctx, 0, 100, None, None).expect("changes");
    let last = page.entries.last().expect("entry");
    assert_eq!(last.payload_state, PayloadState::Absent);
    assert!(last.payload.is_none());
}

#[test]
fn an_erased_payload_keeps_its_position_and_says_so() {
    let mut author = Home::new();
    let id = author.save(BODY, &["sync"]);
    let payload = author.payload(&id);
    let operation = upsert("op-1", &payload);
    let digest = operation.payload_digest.clone().expect("digest");

    let mut peer = Home::new();
    let mut ctx = peer.ctx();
    accept::run(&mut ctx, envelope(vec![operation])).expect("import");
    replica_journal::redact_payload(&peer.conn, &digest, "2026-09-21T12:00:00Z").expect("redact");

    let mut ctx = peer.ctx();
    let page = changes::run(&mut ctx, 0, 100, None, None).expect("changes");
    assert_eq!(page.entries[0].payload_state, PayloadState::Erased);
    assert!(page.entries[0].payload.is_none());
    assert_eq!(
        page.entries[0].payload_digest.as_deref(),
        Some(digest.as_str()),
        "the digest survives as the resurrection barrier"
    );
}

#[test]
fn a_foreign_epoch_fails_the_read_instead_of_returning_an_empty_page() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    let mut ctx = home.ctx();
    let refused = changes::run(&mut ctx, 0, 100, None, Some("0".repeat(32).as_str()));
    assert!(refused.is_err());

    let epoch = home.epoch();
    let mut ctx = home.ctx();
    let accepted = changes::run(&mut ctx, 0, 100, None, Some(&epoch)).expect("changes");
    assert_eq!(accepted.entries.len(), 1);
}

#[test]
fn a_kind_filter_narrows_the_page() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    let mut ctx = home.ctx();
    let memories = changes::run(&mut ctx, 0, 100, Some("memory"), None).expect("changes");
    assert_eq!(memories.entries.len(), 1);

    let mut ctx = home.ctx();
    let documents = changes::run(&mut ctx, 0, 100, Some("document"), None).expect("changes");
    assert!(documents.entries.is_empty());
    assert_eq!(documents.head_sequence, 1);
}
