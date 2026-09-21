#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The notification feed: positions and kinds, never content.

use crate::domains::sync::replica::events;

use crate::domains::sync::replica::test_support as support;

use support::{BODY, Home};

#[test]
fn frames_carry_a_position_and_a_kind_and_nothing_else() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    let mut ctx = home.ctx();
    let frames = events::frames(&mut ctx, 0, 100, None).expect("frames");

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].sequence, 1);
    assert_eq!(frames[0].entity_kind, "memory");
    let encoded = serde_json::to_string(&frames[0]).expect("json");
    assert!(
        !encoded.contains("comemory keeps"),
        "a notification must not carry content: {encoded}"
    );
}

#[test]
fn the_feed_resumes_from_any_position() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    home.save(
        "A second memory about server-ordered replication.",
        &["sync"],
    );

    let mut ctx = home.ctx();
    let all = events::frames(&mut ctx, 0, 100, None).expect("frames");
    assert_eq!(all.len(), 2);

    let mut ctx = home.ctx();
    let resumed = events::frames(&mut ctx, all[0].sequence, 100, None).expect("frames");
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].sequence, all[1].sequence);
}

#[test]
fn a_foreign_epoch_fails_before_any_frame() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    let mut ctx = home.ctx();
    assert!(events::frames(&mut ctx, 0, 100, Some(&"0".repeat(32))).is_err());

    let epoch = home.epoch();
    let mut ctx = home.ctx();
    assert_eq!(
        events::frames(&mut ctx, 0, 100, Some(&epoch))
            .expect("frames")
            .len(),
        1
    );
}
