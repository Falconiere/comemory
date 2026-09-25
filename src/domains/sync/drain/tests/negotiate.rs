#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The negotiation table, every combination of stored selection and manifest
//! outcome: upgrade once, cover an old server partially, never downgrade.

use crate::domains::sync::drain::negotiate::{Decision, Manifest, Protocol, decide};

const STORED: [Option<Protocol>; 3] = [None, Some(Protocol::Legacy), Some(Protocol::Replica)];

fn outcomes() -> Vec<Manifest> {
    vec![
        Manifest::Ready,
        Manifest::Seeding,
        Manifest::Missing,
        Manifest::Unauthorized(403),
        Manifest::Invalid("not a manifest".into()),
        Manifest::Unavailable("HTTP 502".into()),
    ]
}

#[test]
fn a_ready_upstream_selects_replica_and_marks_only_the_first_pass_an_upgrade() {
    for stored in STORED {
        let Decision::Select {
            protocol,
            coverage_reason,
            upgraded,
        } = decide(stored, &Manifest::Ready)
        else {
            panic!("a ready upstream is always selected");
        };
        assert_eq!(protocol, Protocol::Replica);
        assert_eq!(coverage_reason, None);
        assert_eq!(upgraded, stored != Some(Protocol::Replica), "{stored:?}");
    }
}

#[test]
fn an_old_or_seeding_upstream_is_covered_partially_until_it_is_ready() {
    for stored in [None, Some(Protocol::Legacy)] {
        assert_eq!(
            decide(stored, &Manifest::Seeding),
            Decision::Select {
                protocol: Protocol::Legacy,
                coverage_reason: Some("upstream_not_ready"),
                upgraded: false,
            }
        );
        assert_eq!(
            decide(stored, &Manifest::Missing),
            Decision::Select {
                protocol: Protocol::Legacy,
                coverage_reason: Some("replica_unsupported"),
                upgraded: false,
            }
        );
    }
}

#[test]
fn a_replica_key_is_never_downgraded_whatever_the_upstream_answers() {
    for manifest in outcomes() {
        let decision = decide(Some(Protocol::Replica), &manifest);
        if let Decision::Select { protocol, .. } = decision {
            assert_eq!(protocol, Protocol::Replica, "{manifest:?}");
        }
    }
    assert!(matches!(
        decide(Some(Protocol::Replica), &Manifest::Missing),
        Decision::ProtocolError(_)
    ));
    assert!(matches!(
        decide(Some(Protocol::Replica), &Manifest::Seeding),
        Decision::Unavailable(_)
    ));
}

#[test]
fn auth_and_protocol_failures_never_select_anything() {
    for stored in STORED {
        for status in [401, 403] {
            assert_eq!(
                decide(stored, &Manifest::Unauthorized(status)),
                Decision::Suspend(status),
                "the refusal's own status is kept"
            );
        }
        assert!(matches!(
            decide(stored, &Manifest::Invalid("x".into())),
            Decision::ProtocolError(_)
        ));
        assert!(matches!(
            decide(stored, &Manifest::Unavailable("x".into())),
            Decision::Unavailable(_)
        ));
    }
}

#[test]
fn stored_literals_round_trip() {
    for protocol in [Protocol::Legacy, Protocol::Replica] {
        assert_eq!(Protocol::parse(Some(protocol.as_str())), Some(protocol));
    }
    assert_eq!(Protocol::parse(None), None);
    assert_eq!(Protocol::parse(Some("replica-v9")), None);
}
