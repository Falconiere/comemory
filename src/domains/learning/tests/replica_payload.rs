#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `FeedbackEventV1`'s identity rules and canonical bytes.

use crate::domains::learning::replica_payload::{
    EVENT_ID_PREFIX, FeedbackEventV1, Target, is_minted_id, mint_event_id,
};

const DEVICE: &str = "9a1e0c2b4d6f8a1e0c2b4d6f8a1e0c2b";
const EVENT: &str = "ev-4f0c0123456789abcdef0123456789ab";

fn verdict() -> FeedbackEventV1 {
    FeedbackEventV1 {
        event_id: EVENT.to_string(),
        device: DEVICE.to_string(),
        at: "2026-09-24T10:00:00.123456789Z".to_string(),
        verdict: "used".to_string(),
        provenance: "manual".to_string(),
        surface: Some("mcp".to_string()),
        actor: Some("claude-code/2.1.0".to_string()),
        origin_query_id: format!("{DEVICE}:q-20260924-1a2b3c4d"),
        target: Target::Code {
            repo: "Falconiere/comemory".to_string(),
            path: "src/store/feedback.rs".to_string(),
            symbol: "upsert_used".to_string(),
            version: Some("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".to_string()),
        },
    }
}

#[test]
fn a_well_formed_verdict_holds_under_its_own_id_only() {
    assert!(verdict().holds_for(EVENT));
    assert!(!verdict().holds_for("ev-ffffffffffffffffffffffffffffffff"));
}

#[test]
fn each_broken_rule_is_refused() {
    let mut coactivation = verdict();
    coactivation.provenance = "auto_coactivation".to_string();
    let mut unnamespaced = verdict();
    unnamespaced.origin_query_id = "q-20260924-1a2b3c4d".to_string();
    let mut foreign = verdict();
    foreign.origin_query_id = format!("{}:q-1", "0".repeat(32));
    let mut surface = verdict();
    surface.surface = Some("sync".to_string());
    let mut when = verdict();
    when.at = "yesterday".to_string();
    let mut verdict_word = verdict();
    verdict_word.verdict = "liked".to_string();
    for (name, broken) in [
        ("an automatic reward that stays local", coactivation),
        ("a query id no device namespaced", unnamespaced),
        ("a query id another device namespaced", foreign),
        ("an unknown surface", surface),
        ("an unparsable time", when),
        ("an unknown verdict", verdict_word),
    ] {
        assert!(!broken.holds_for(EVENT), "{name} must be refused");
    }
}

#[test]
fn the_canonical_bytes_are_stable_and_reject_unknown_fields_on_decode() {
    let (bytes, digest) = verdict().canonical().expect("canonical");
    assert_eq!(
        verdict().canonical().expect("again"),
        (bytes.clone(), digest)
    );
    let decoded: FeedbackEventV1 = serde_json::from_str(&bytes).expect("decode");
    assert_eq!(decoded, verdict());
    let mut extra: serde_json::Value = serde_json::from_str(&bytes).expect("json");
    extra["title"] = serde_json::json!("memory content has no place here");
    assert!(serde_json::from_value::<FeedbackEventV1>(extra).is_err());
    let mut smuggled: serde_json::Value = serde_json::from_str(&bytes).expect("json");
    smuggled["target"]["snippet"] = serde_json::json!("fn upsert_used() {}");
    assert!(
        serde_json::from_value::<FeedbackEventV1>(smuggled).is_err(),
        "a target carries no source"
    );
}

#[test]
fn minted_ids_have_the_event_shape_and_never_repeat() {
    let a = mint_event_id().expect("a");
    let b = mint_event_id().expect("b");
    assert!(is_minted_id(&a, EVENT_ID_PREFIX) && is_minted_id(&b, EVENT_ID_PREFIX));
    assert_ne!(a, b);
    assert!(is_minted_id(DEVICE, ""));
    assert!(!is_minted_id(&DEVICE.to_uppercase(), ""));
    assert!(
        !is_minted_id(DEVICE, EVENT_ID_PREFIX),
        "a device id is not an event id"
    );
}
