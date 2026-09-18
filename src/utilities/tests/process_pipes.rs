#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for the worker-report polling helper.
//!
//! The dangerous case is a channel that closes without a message: folding it
//! into a value is what stops the drain loop spinning to the deadline for a
//! worker that will never speak again. A real `mpsc` channel with a really
//! dropped `Sender` reproduces it exactly.

use std::sync::mpsc;

use comemory::utilities::process_pipes::poll;

#[test]
fn a_pending_channel_leaves_the_slot_empty() {
    let (tx, rx) = mpsc::channel::<u8>();
    let mut slot = None;
    poll(&rx, &mut slot, || 99);
    assert_eq!(slot, None, "nothing sent yet, and no fallback invented");
    drop(tx);
}

#[test]
fn a_reported_value_fills_the_slot_once() {
    let (tx, rx) = mpsc::channel::<u8>();
    tx.send(7).unwrap();
    tx.send(8).unwrap();
    let mut slot = None;
    poll(&rx, &mut slot, || 99);
    assert_eq!(slot, Some(7));
    // A filled slot is never overwritten, so a second report cannot replace
    // the first worker verdict the drain loop already acted on.
    poll(&rx, &mut slot, || 99);
    assert_eq!(slot, Some(7));
}

#[test]
fn a_closed_channel_folds_into_the_fallback() {
    let (tx, rx) = mpsc::channel::<u8>();
    drop(tx);
    let mut slot = None;
    poll(&rx, &mut slot, || 99);
    assert_eq!(
        slot,
        Some(99),
        "a worker that ended without reporting must still complete its slot"
    );
}

#[test]
fn a_value_sent_before_the_sender_dropped_still_arrives() {
    // Disconnected must not shadow a buffered value: mpsc yields the payload
    // first, so a worker that reported and then exited is read correctly.
    let (tx, rx) = mpsc::channel::<u8>();
    tx.send(5).unwrap();
    drop(tx);
    let mut slot = None;
    poll(&rx, &mut slot, || 99);
    assert_eq!(slot, Some(5));
}
