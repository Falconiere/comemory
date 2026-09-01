#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `GET|PUT /api/v1/config/retrieval` driven through the real router
//! (`tests/common/serve_state.rs`) — console-api spec AC-14: an
//! out-of-range knob is `400` and leaves `config.toml` byte-identical;
//! a valid one persists, reloads `AppState.cfg`, and the next `GET` echoes
//! it.

use comemory::config::Paths;
use serde_json::json;

use crate::test_common::serve_state;

#[tokio::test]
async fn get_reports_the_live_knobs_and_their_ranges() {
    let session = serve_state::session(false);

    let resp = serve_state::send(&session, "GET", "/api/v1/config/retrieval", None).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["meta"]["command"], "config.retrieval");
    let data = &resp.json["data"];
    assert_eq!(data["rrf_k"].as_f64(), Some(60.0));
    assert_eq!(data["decay"].as_f64(), Some(0.5));
    assert_eq!(data["mmr_lambda"].as_f64(), Some(0.7));
    assert_eq!(data["graph_hops"].as_u64(), Some(2));
    assert_eq!(data["graph_seeds"].as_u64(), Some(8));
    assert_eq!(data["top_k"].as_u64(), Some(12));
    assert_eq!(data["bm25_weights"][1].as_f64(), Some(3.0));
    assert_eq!(data["prior_clamp"][0].as_f64(), Some(0.5));
    assert_eq!(data["ranges"]["graph_hops"]["max"].as_f64(), Some(4.0));
    assert_eq!(data["ranges"]["mmr_lambda"]["max"].as_f64(), Some(1.0));
    assert!(
        data["ranges"]["rrf_k"]["note"]
            .as_str()
            .is_some_and(|n| !n.is_empty()),
        "every range carries the part a number cannot express"
    );
}

#[tokio::test]
async fn an_invalid_knob_is_400_and_leaves_config_toml_byte_identical_ac14() {
    let session = serve_state::session(false);
    let config_file = Paths::new(session.home.path()).config_file();
    let original = "[retrieval]\nrrf_k = 45.0\n";
    std::fs::write(&config_file, original).expect("seed config.toml");

    let resp = serve_state::send(
        &session,
        "PUT",
        "/api/v1/config/retrieval",
        Some(json!({ "rrf_k": 0 })),
    )
    .await;
    assert_eq!(resp.status, 400, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], "bad_request");
    assert!(
        resp.json["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("rrf_k")),
        "the validator's message names the knob: {}",
        resp.text
    );

    let after = std::fs::read_to_string(&config_file).expect("config.toml");
    assert_eq!(after, original, "AC-14: a rejected PUT writes nothing");
}

#[tokio::test]
async fn a_valid_knob_persists_reloads_and_is_echoed_by_the_next_get_ac14() {
    let session = serve_state::session(false);
    let config_file = Paths::new(session.home.path()).config_file();

    let resp = serve_state::send(
        &session,
        "PUT",
        "/api/v1/config/retrieval",
        Some(json!({ "rrf_k": 30, "graph_hops": 1 })),
    )
    .await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["meta"]["command"], "config.retrieval.update");
    assert_eq!(resp.json["data"]["rrf_k"].as_f64(), Some(30.0));
    assert_eq!(resp.json["data"]["graph_hops"].as_u64(), Some(1));

    let written = std::fs::read_to_string(&config_file).expect("config.toml was written");
    assert!(written.contains("rrf_k = 30.0"), "got:\n{written}");
    assert!(
        !written.contains("top_k"),
        "only the supplied keys are written:\n{written}"
    );

    // A following GET reads `AppState.cfg`, so echoing 30 proves the
    // in-memory config was reloaded rather than just the file rewritten.
    let resp = serve_state::send(&session, "GET", "/api/v1/config/retrieval", None).await;
    assert_eq!(resp.json["data"]["rrf_k"].as_f64(), Some(30.0));
    assert_eq!(resp.json["data"]["graph_hops"].as_u64(), Some(1));
    assert_eq!(
        resp.json["data"]["top_k"].as_u64(),
        Some(12),
        "an untouched knob keeps its default"
    );
}

#[tokio::test]
async fn an_unknown_body_field_is_rejected_and_a_read_only_server_refuses_the_put() {
    let session = serve_state::session(false);
    let unknown = serve_state::send(
        &session,
        "PUT",
        "/api/v1/config/retrieval",
        Some(json!({ "rrf_kk": 30 })),
    )
    .await;
    assert!(
        unknown.status.is_client_error(),
        "deny_unknown_fields must reject a typo'd knob, got {} / {}",
        unknown.status,
        unknown.text
    );

    let read_only = serve_state::session(true);
    let resp = serve_state::send(
        &read_only,
        "PUT",
        "/api/v1/config/retrieval",
        Some(json!({ "rrf_k": 30 })),
    )
    .await;
    assert_eq!(resp.status, 405, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], "read_only");
    assert!(
        !Paths::new(read_only.home.path()).config_file().exists(),
        "a refused PUT writes nothing"
    );

    // The read half stays available on a read-only server.
    let resp = serve_state::send(&read_only, "GET", "/api/v1/config/retrieval", None).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
}
