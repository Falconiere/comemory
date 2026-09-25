#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The transport against a REAL engine on a loopback socket: a real manifest
//! decodes, a wrong path is `NotFound`, a wrong credential is `Auth`, a `2xx`
//! that is not the promised answer is `Protocol`, and a managed origin that
//! does not echo the negotiated protocol is refused.

use std::time::Duration;

use serde_json::Value;

use crate::domains::sync::drain::test_support::LiveEngine;
use crate::domains::sync::drain::transport::{Failure, Transport};
use crate::domains::sync::replica::contract_views::ManifestResponse;

fn transport(api_url: &str, secret: &str) -> Transport {
    Transport::new(api_url, secret, Duration::from_secs(10)).expect("transport")
}

#[test]
fn a_real_manifest_decodes() {
    let engine = LiveEngine::start();
    let manifest: ManifestResponse = transport(&engine.api_url, &engine.token())
        .get("/v1/sync/replica/manifest", &[])
        .expect("manifest");
    assert_eq!(manifest.protocol, "replica-v1");
    assert_eq!(manifest.stream_epoch.len(), 32);
}

#[test]
fn a_wrong_path_is_not_found_and_a_wrong_credential_is_auth() {
    let engine = LiveEngine::start();
    let missing = transport(&format!("{}/nope", engine.api_url), &engine.token())
        .get::<Value>("/v1/sync/replica/manifest", &[]);
    assert_eq!(missing.err(), Some(Failure::NotFound));
    let refused =
        transport(&engine.api_url, "not-the-token").get::<Value>("/v1/sync/replica/manifest", &[]);
    assert_eq!(refused.err(), Some(Failure::Auth(401)));
}

#[test]
fn a_success_that_is_not_the_promised_answer_is_a_protocol_failure() {
    let engine = LiveEngine::start();
    // The stats route answers 200 with an envelope that is not a manifest.
    let wrong =
        transport(&engine.api_url, &engine.token()).get::<ManifestResponse>("/v1/stats", &[]);
    assert!(matches!(wrong, Err(Failure::Protocol(_))), "{wrong:?}");
}

#[test]
fn a_managed_origin_must_echo_the_protocol_it_was_asked_under() {
    let engine = LiveEngine::start();
    // A bare engine echoes nothing: as a MANAGED origin it would be lying.
    let answer = transport(&engine.api_url, &engine.token())
        .managed("replica-v1", 3)
        .get::<ManifestResponse>("/v1/sync/replica/manifest", &[]);
    assert!(matches!(answer, Err(Failure::Protocol(_))), "{answer:?}");
}

#[test]
fn an_unreachable_origin_is_unavailable() {
    let answer =
        transport("http://127.0.0.1:1/api", "x").get::<Value>("/v1/sync/replica/manifest", &[]);
    assert!(matches!(answer, Err(Failure::Unavailable(_))), "{answer:?}");
}

#[test]
fn a_conflict_carries_the_engines_code() {
    let engine = LiveEngine::start();
    let ahead = transport(&engine.api_url, &engine.token())
        .get::<Value>("/v1/sync/replica/changes", &[("since", "99".to_string())]);
    assert_eq!(
        ahead.err(),
        Some(Failure::Conflict("cursor_ahead".to_string()))
    );
    let foreign = transport(&engine.api_url, &engine.token()).get::<Value>(
        "/v1/sync/replica/changes",
        &[("since", "0".to_string()), ("epoch", "0".repeat(32))],
    );
    assert_eq!(
        foreign.err(),
        Some(Failure::Conflict("epoch_mismatch".to_string()))
    );
}
