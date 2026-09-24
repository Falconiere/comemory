#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! What a shared run's summary may carry, built from the summaries the real
//! cores store.

use crate::domains::sync::replica::activity_payload::{ActivityEventV1, shared_keys};
use crate::store::activity::ActivityRow;
use serde_json::json;

const DEVICE: &str = "9a1e0c2b4d6f8a1e0c2b4d6f8a1e0c2b";
const EVENT: &str = "ev-4f0c0123456789abcdef0123456789ab";

fn row(command: &str, summary: &str) -> ActivityRow {
    ActivityRow {
        id: 7,
        at: "2026-09-24T10:00:00.123456789Z".to_string(),
        command: command.to_string(),
        source: "mcp".to_string(),
        actor: Some("claude-code/2.1.0".to_string()),
        repo: Some("demo".to_string()),
        duration_ms: 37,
        ok: true,
        error_code: None,
        summary: Some(summary.to_string()),
        device: None,
    }
}

#[test]
fn a_find_shares_its_counts_and_query_but_not_its_local_ids() {
    // The exact summary `retrieval::find` stores.
    let stored = r#"{"hits":{"memory":1},"query":"upsert used feedback counter",
        "query_id":"q-20260924-fff92be0","top":["c2468e46"],"total":1}"#;
    let event =
        ActivityEventV1::from_row(&row("find", stored), "Falconiere/comemory", DEVICE, EVENT)
            .expect("find is shared");
    assert_eq!(
        event.summary,
        Some(json!({"hits": {"memory": 1}, "query": "upsert used feedback counter", "total": 1}))
    );
    assert_eq!(
        event.repo, "Falconiere/comemory",
        "the canonical scope, not the label"
    );
    assert!(event.holds_for(EVENT));
}

#[test]
fn a_save_never_shares_its_title_and_a_secret_query_is_withheld() {
    let stored = r#"{"created":true,"duplicate_of":null,"id":"c2468e46","kind":"decision",
        "supersedes":0,"tags":0,"title":"memory content"}"#;
    let save =
        ActivityEventV1::from_row(&row("save", stored), "Falconiere/comemory", DEVICE, EVENT)
            .expect("save");
    assert_eq!(
        save.summary,
        Some(json!({"id": "c2468e46", "kind": "decision", "supersedes": 0, "tags": 0}))
    );
    let secret = format!(
        r#"{{"hits":0,"lang":null,"query":"rotate AKIA{}{} now"}}"#,
        "IOSFODNN7", "EXAMPLE"
    );
    let code = ActivityEventV1::from_row(&row("search-code", &secret), "r/r", DEVICE, EVENT)
        .expect("search-code");
    assert_eq!(
        code.summary,
        Some(json!({"hits": 0, "lang": null, "query_withheld": true}))
    );
}

#[test]
fn bookkeeping_is_never_shared_and_a_smuggled_key_is_refused() {
    assert!(shared_keys("sync.import").is_none());
    assert!(ActivityEventV1::from_row(&row("sync.import", "{}"), "r/r", DEVICE, EVENT).is_none());

    let mut event = ActivityEventV1::from_row(
        &row("find", r#"{"hits":{},"total":0}"#),
        "r/r",
        DEVICE,
        EVENT,
    )
    .expect("find");
    assert!(event.holds_for(EVENT));
    event.summary = Some(json!({"hits": {}, "total": 0, "top": ["c2468e46"]}));
    assert!(
        !event.holds_for(EVENT),
        "the receiver checks the same allowlist"
    );
    event.summary = Some(json!("a summary that is not an object"));
    assert!(!event.holds_for(EVENT));
}
