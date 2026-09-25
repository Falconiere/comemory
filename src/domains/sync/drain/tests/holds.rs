#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Hold reconsideration over a real store: a hold comes due only when its
//! cause is gone, the cursor rewinds to one before the earliest due hold, and
//! due holds are dropped while the rest stay.

use crate::domains::sync::drain::holds::reconsider;
use crate::domains::sync::drain::test_support::approve;
use crate::domains::sync::replica::test_support::Home;
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::store::replica_cursor::{self, Cursor};
use crate::store::replica_outbox::{self, Outcome, Scope};
use crate::store::replica_pull_hold::{self, PullHold};
use crate::store::sync_exchange::ExchangeKey;

const EPOCH: &str = "epoch-holds";

fn key() -> ExchangeKey {
    ExchangeKey::new("http://127.0.0.1:9/api", "ws")
}

fn hold(at: i64, reason: &str, entity_key: &str, repository: Option<&str>) -> PullHold {
    PullHold {
        from_sequence: at,
        to_sequence: at,
        reason: reason.into(),
        entity_kind: Some("memory".into()),
        entity_key: Some(entity_key.into()),
        repository: repository.map(str::to_string),
        policy_revision: (reason == "server_withheld").then_some(1),
    }
}

fn cursor_at(sequence: i64) -> Cursor {
    let key = key();
    Cursor {
        api_url: key.api_url,
        workspace_id: key.workspace_id,
        stream_epoch: EPOCH.into(),
        applied_sequence: sequence,
        anchor: None,
    }
}

fn remaining(home: &Home) -> Vec<i64> {
    replica_pull_hold::list(&home.conn, &key(), EPOCH)
        .expect("list")
        .iter()
        .map(|h| h.from_sequence)
        .collect()
}

#[test]
fn a_hold_comes_due_only_when_its_cause_is_gone() {
    let mut home = Home::new();
    let key = key();
    // A real save leaves a real owed upload for its memory.
    let owed = home.save("A local edit still owed upstream.", &[]);
    for h in [
        hold(4, "policy", "m4", Some("acme/private")),
        hold(6, "pending_local", &owed, None),
        hold(8, "server_withheld", "m8", None),
    ] {
        replica_pull_hold::record(&home.conn, &key, EPOCH, &h, "t").expect("record");
    }
    // Revision 1, acme/private unapproved, the memory still owes: none due.
    let unchanged = approve(&home.conn, &key, &["falconiere/comemory"]);
    let mut cursor = cursor_at(10);
    assert_eq!(
        reconsider(&home.conn, &key, &unchanged, &mut cursor, "t").expect("r"),
        0
    );
    assert_eq!(
        cursor.applied_sequence, 10,
        "nothing is due, nothing rewinds"
    );

    // The owed upload settles: the pending_local hold is due, and only it.
    for row in replica_outbox::read(&home.conn, Scope::All, 10).expect("owed") {
        let accepted = Outcome::Accepted {
            sequence: Some(11),
            disposition: "accepted",
            epoch: Some(EPOCH),
        };
        replica_outbox::record(&home.conn, &row.operation_id, accepted, "t").expect("settle");
    }
    assert_eq!(
        reconsider(&home.conn, &key, &unchanged, &mut cursor, "t").expect("r"),
        1
    );
    assert_eq!(
        cursor.applied_sequence, 5,
        "one before the earliest due hold"
    );
    assert_eq!(
        replica_cursor::load(&home.conn, &key.api_url, &key.workspace_id)
            .expect("load")
            .map(|c| c.applied_sequence),
        Some(5),
        "the rewind is durable"
    );
    assert_eq!(remaining(&home), vec![4, 8]);

    // Approval restored under a new policy revision: both remaining come due.
    approve(&home.conn, &key, &["falconiere/comemory", "acme/private"]);
    home.conn
        .execute("UPDATE sync_policy_snapshot SET revision = 2", [])
        .expect("revise");
    let revised = RepositoryPolicy::from_snapshot(&home.conn, &key).expect("policy");
    let mut cursor = cursor_at(10);
    assert_eq!(
        reconsider(&home.conn, &key, &revised, &mut cursor, "t").expect("r"),
        2
    );
    assert_eq!(cursor.applied_sequence, 3);
    assert!(remaining(&home).is_empty());
}

#[test]
fn a_cursor_with_no_stream_yet_has_nothing_to_reconsider() {
    let home = Home::new();
    let key = key();
    let h = hold(4, "pending_local", "m4", None);
    replica_pull_hold::record(&home.conn, &key, EPOCH, &h, "t").expect("record");
    let policy = approve(&home.conn, &key, &[]);
    let mut cursor = cursor_at(0);
    cursor.stream_epoch.clear();

    assert_eq!(
        reconsider(&home.conn, &key, &policy, &mut cursor, "t").expect("r"),
        0
    );
    assert_eq!(remaining(&home), vec![4]);
}
