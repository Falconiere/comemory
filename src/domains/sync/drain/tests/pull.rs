#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! One pull page against a REAL engine: entries apply in upstream order, the
//! cursor takes the page's raw continuation, each applied entity is bound,
//! and a page a filter emptied still advances — recording the skipped
//! positions as `server_withheld` only on a managed origin.

use super::{Paged, Pull, Step, page, walk};
use crate::domains::sync::drain::test_support::{LiveEngine, approve, push_all};
use crate::domains::sync::replica::test_support::{BODY, Home};
use crate::store::replica_binding;
use crate::store::replica_cursor::{self, Cursor};
use crate::store::replica_pull_hold;

const REPO: &str = "falconiere/comemory";

fn fresh_cursor(engine: &LiveEngine) -> Cursor {
    Cursor {
        api_url: engine.api_url.clone(),
        workspace_id: "ws".into(),
        stream_epoch: String::new(),
        applied_sequence: 0,
        anchor: None,
    }
}

#[test]
fn a_page_applies_in_order_binds_and_takes_the_continuation() {
    let engine = LiveEngine::start();
    let mut writer = Home::new();
    let first = writer.save(BODY, &["sync"]);
    writer.save("A second memory the reader pulls.", &["sync"]);
    writer.save("A third memory the reader pulls.", &["sync"]);
    push_all(&writer, &engine);
    let mut reader = Home::new();
    let key = engine.key();
    let policy = approve(&reader.conn, &key, &[REPO]);
    let transport = engine.transport();
    let pull = Pull {
        key: &key,
        transport: &transport,
        policy: &policy,
        managed_revision: None,
    };
    let (paths, cfg) = (reader.paths.clone(), reader.cfg.clone());
    let step = Step {
        paths: &paths,
        cfg: &cfg,
        pull: &pull,
        at: "t",
    };
    let mut cursor = fresh_cursor(&engine);

    let paged = page(&mut reader.conn, &step, &mut cursor).expect("page");

    assert_eq!((paged.applied, paged.head), (3, 3), "{paged:?}");
    assert!(paged.advanced && paged.stalled.is_none());
    assert_eq!(cursor.applied_sequence, 3);
    assert_eq!(
        replica_cursor::load(&reader.conn, &engine.api_url, "ws")
            .expect("load")
            .map(|c| c.applied_sequence),
        Some(3),
        "the cursor is durable"
    );
    let bound = replica_binding::get(&reader.conn, &key, "memory", &first)
        .expect("binding")
        .expect("bound");
    assert_eq!(bound.synced_sequence, Some(1));

    let again = page(&mut reader.conn, &step, &mut cursor).expect("again");
    assert_eq!(again.applied, 0);
    assert!(!again.advanced, "nothing above the head");
}

#[test]
fn a_filtered_empty_page_advances_and_records_withheld_positions_only_when_managed() {
    let engine = LiveEngine::start();
    let mut writer = Home::new();
    writer.save(BODY, &["sync"]);
    writer.save("Another memory a filtered page skips.", &["sync"]);
    push_all(&writer, &engine);
    // A page filtered to a kind the feed does not hold: no entries, and the
    // raw continuation names the last position scanned.
    let filtered = engine.changes(0, Some("document_revision"));
    assert!(filtered.entries.is_empty());
    assert_eq!(filtered.next_sequence, Some(2));

    for (managed_revision, withheld) in [(None, 0), (Some(3), 1)] {
        let mut reader = Home::new();
        let key = engine.key();
        let policy = approve(&reader.conn, &key, &[REPO]);
        let transport = engine.transport();
        let pull = Pull {
            key: &key,
            transport: &transport,
            policy: &policy,
            managed_revision,
        };
        let (paths, cfg) = (reader.paths.clone(), reader.cfg.clone());
        let step = Step {
            paths: &paths,
            cfg: &cfg,
            pull: &pull,
            at: "t",
        };
        let mut cursor = fresh_cursor(&engine);
        cursor.stream_epoch.clone_from(&filtered.stream_epoch);
        let mut paged = Paged::default();

        walk(&mut reader.conn, &step, &mut cursor, &filtered, &mut paged).expect("walk");

        assert_eq!(cursor.applied_sequence, 2, "the empty page still advances");
        let holds =
            replica_pull_hold::list(&reader.conn, &key, &filtered.stream_epoch).expect("holds");
        assert_eq!(holds.len(), withheld, "{managed_revision:?}: {holds:?}");
        if let Some(hold) = holds.first() {
            assert_eq!(hold.reason, "server_withheld");
            assert_eq!((hold.from_sequence, hold.to_sequence), (1, 2));
            assert_eq!(hold.policy_revision, Some(3));
        }
    }
}
