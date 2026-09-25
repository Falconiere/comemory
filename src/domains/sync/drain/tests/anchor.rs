#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The anchor check against a REAL engine: the entry at the cursor's position
//! is re-read and compared with the operation the cursor was left on.

use crate::domains::sync::drain::anchor::{Check, check};
use crate::domains::sync::drain::test_support::{LiveEngine, push_all};
use crate::domains::sync::replica::test_support::{BODY, Home};
use crate::store::replica_cursor::{Anchor, Cursor};

#[test]
fn the_anchor_reads_intact_rewritten_or_unknown() {
    let engine = LiveEngine::start();
    let mut writer = Home::new();
    writer.save(BODY, &["sync"]);
    writer.save(
        "A second memory, the entry the cursor is left on.",
        &["sync"],
    );
    push_all(&writer, &engine);
    let page = engine.changes(0, None);
    let at_two = page
        .entries
        .iter()
        .find(|e| e.sequence == 2)
        .expect("position 2");
    let cursor = |operation_id: Option<&str>| Cursor {
        api_url: engine.api_url.clone(),
        workspace_id: "ws".into(),
        stream_epoch: page.stream_epoch.clone(),
        applied_sequence: 2,
        anchor: operation_id.map(|id| Anchor {
            sequence: 2,
            operation_id: id.to_string(),
        }),
    };
    let transport = engine.transport();

    assert_eq!(
        check(&transport, &cursor(Some(&at_two.operation_id))),
        Ok(Check::Intact)
    );
    assert_eq!(
        check(
            &transport,
            &cursor(Some("op-20260924-someoneelseswrite0000000000001"))
        ),
        Ok(Check::Rewritten),
        "the same position names another operation: the stream was rewritten"
    );
    assert_eq!(check(&transport, &cursor(None)), Ok(Check::Unknown));
}
