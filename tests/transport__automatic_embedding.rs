#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Real inference through HTTP and a freshly launched MCP stdio server.

#[path = "common/mcp_bin.rs"]
mod mcp_bin;
#[path = "common/serve_bin.rs"]
mod serve_bin;

use serde_json::json;

const BODY: &str = include_str!("../docs/guides/byo-vectors.md");
const QUERY: &str = "How can I add semantic vectors from an external model?";

#[test]
#[ignore = "requires COMEMORY_TEST_EMBED_CMD with a real 1024-dimensional model"]
fn real_provider_http_save_search_and_update() {
    let cmd = std::env::var("COMEMORY_TEST_EMBED_CMD").expect("provider");
    let server = serve_bin::ServeHome::with_args_env(&[], &[("COMEMORY_EMBED_CMD", &cmd)]);
    let saved = server.post("/memories", &json!({"body": BODY}));
    assert!(saved.get("warnings").is_none());
    let result = server.get_q("/memories/search", &[("query", QUERY)]);
    assert!(
        result["hits"][0]["score_parts"]["legs"]["ann"]
            .as_f64()
            .is_some()
    );
    let id = saved["id"].as_str().expect("id");
    let updated = server.patch(
        &format!("/memories/{id}"),
        &json!({"body": format!("{BODY}\nAdditional verified note.")}),
    );
    assert_eq!(updated["superseded"], id);
    assert!(updated.get("warnings").is_none());
}

#[tokio::test]
#[ignore = "requires COMEMORY_TEST_EMBED_CMD with a real 1024-dimensional model"]
async fn real_provider_mcp_save_and_search() {
    let cmd = std::env::var("COMEMORY_TEST_EMBED_CMD").expect("provider");
    let cwd = tempfile::tempdir().expect("cwd");
    let mcp =
        mcp_bin::McpHome::spawn_with_env(cwd.path(), &[], &[("COMEMORY_EMBED_CMD", &cmd)]).await;
    let saved = mcp
        .data("save", json!({"body": BODY, "repo": "inference-test"}))
        .await;
    assert!(saved.get("warnings").is_none());
    let result = mcp
        .data("search", json!({"query": QUERY, "repo": "inference-test"}))
        .await;
    assert!(
        result["hits"][0]["score_parts"]["legs"]["ann"]
            .as_f64()
            .is_some()
    );
    mcp.cancel().await;
}

#[test]
fn slow_provider_does_not_hold_shared_http_connection() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let marker = scratch.path().join("provider-started");
    let cmd = format!("touch '{}'; sleep 4; exit 1", marker.display());
    let server = serve_bin::ServeHome::with_args_env(&[], &[("COMEMORY_EMBED_CMD", &cmd)]);
    std::thread::scope(|scope| {
        let pending = scope.spawn(|| {
            server.post(
                "/memories",
                &json!({"body": "Slow embedding should not block reads"}),
            )
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(marker.exists(), "provider did not start");
        let started = std::time::Instant::now();
        server.get("/memories");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "read waited for provider"
        );
        assert!(pending.join().expect("save").get("warnings").is_some());
    });
}

#[test]
fn failed_http_provider_warns_on_save_and_body_update() {
    let server = serve_bin::ServeHome::with_args_env(&[], &[("COMEMORY_EMBED_CMD", "exit 1")]);
    let saved = server.post(
        "/memories",
        &json!({"title": "Recovery", "body": "Original body"}),
    );
    assert!(
        saved["warnings"]
            .as_array()
            .is_some_and(|warnings| !warnings.is_empty())
    );
    assert!(std::path::Path::new(saved["path"].as_str().expect("path")).exists());
    let id = saved["id"].as_str().expect("id");
    let updated = server.patch(&format!("/memories/{id}"), &json!({"body": "Revised body"}));
    assert_eq!(updated["superseded"], id);
    assert!(
        updated["warnings"]
            .as_array()
            .is_some_and(|warnings| !warnings.is_empty())
    );
}
