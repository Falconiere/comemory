#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! What a real `comemory mcp` session records in `activity_log`: the tool
//! call's own row, and the `actor` taken from the `clientInfo` the host sent
//! at `initialize`.
//!
//! The client here is the REAL rmcp client speaking to the REAL binary over
//! its stdin/stdout — the only way to prove the handshake actually carries
//! the label a row later reports.

use std::path::Path;
use std::time::Duration;

use assert_cmd::cargo::cargo_bin;
use comemory::store::activity::{ActivityFilter, ActivityRow, list};
use comemory::store::connection;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt, service::RunningService};
use serde_json::{Value, json};
use tempfile::TempDir;

/// Bound on one protocol round-trip, matching `tests/common/mcp_bin.rs`.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// A client that names itself `name/version` at `initialize`.
fn client_named(name: &str, version: &str) -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new(name, version),
    )
}

/// Spawn `comemory mcp` on `data_dir` and complete the handshake as `client`.
async fn connect(
    data_dir: &Path,
    cwd: &Path,
    client: ClientConfig,
) -> RunningService<RoleClient, ClientConfig> {
    let mut cmd = tokio::process::Command::new(cargo_bin("comemory"));
    cmd.arg("mcp")
        .env("COMEMORY_DATA_DIR", data_dir)
        .current_dir(cwd);
    let transport = TokioChildProcess::new(cmd).expect("spawn comemory mcp");
    tokio::time::timeout(CALL_TIMEOUT, client.serve(transport))
        .await
        .expect("initialize did not answer")
        .expect("initialize handshake")
}

/// Call one tool and return its structured content, failing loudly.
async fn call(
    session: &RunningService<RoleClient, ClientConfig>,
    name: &str,
    arguments: Value,
) -> Value {
    let Value::Object(map) = arguments else {
        panic!("tool arguments must be a JSON object");
    };
    let params = CallToolRequestParams::new(name.to_string()).with_arguments(map);
    let result = tokio::time::timeout(CALL_TIMEOUT, session.call_tool(params))
        .await
        .expect("tools/call did not answer")
        .expect("tools/call failed at the protocol level");
    assert_ne!(
        result.is_error,
        Some(true),
        "tool `{name}` failed: {:?}",
        result.structured_content
    );
    result
        .structured_content
        .unwrap_or_else(|| panic!("tool `{name}` returned no structured content"))
}

fn rows(data_dir: &Path) -> Vec<ActivityRow> {
    let conn = connection::open(data_dir.join("comemory.db")).expect("open db");
    list(&conn, &ActivityFilter::default(), 0, 0)
        .expect("list activity")
        .0
}

#[tokio::test]
async fn a_save_through_mcp_records_the_hosts_client_info_as_its_actor() {
    let home = TempDir::new().unwrap();
    let data_dir = home.path().join(".comemory");
    let session = connect(&data_dir, home.path(), client_named("test-host", "1.0")).await;

    let saved = call(
        &session,
        "save",
        json!({
            "body": "pgbouncer in transaction mode fixes pool exhaustion",
            "repo": "demo",
            "kind": "decision",
        }),
    )
    .await;
    let id = saved["id"].as_str().expect("the tool returns an id");

    session.cancel().await.expect("close the session");

    let recorded = rows(&data_dir);
    let save_row = recorded
        .iter()
        .find(|r| r.command == "save")
        .expect("a save row");
    assert_eq!(save_row.source, "mcp");
    assert_eq!(
        save_row.actor.as_deref(),
        Some("test-host/1.0"),
        "the actor is the clientInfo the host sent at initialize"
    );
    assert_eq!(save_row.repo.as_deref(), Some("demo"));
    let summary: Value =
        serde_json::from_str(save_row.summary.as_deref().expect("a summary")).unwrap();
    assert_eq!(summary["id"], id);
}

#[tokio::test]
async fn a_find_through_mcp_records_its_own_row_beside_the_save() {
    let home = TempDir::new().unwrap();
    let data_dir = home.path().join(".comemory");
    let session = connect(&data_dir, home.path(), client_named("agent-x", "9.9")).await;

    call(
        &session,
        "save",
        json!({"body": "pgbouncer pool exhaustion", "repo": "demo", "kind": "note"}),
    )
    .await;
    call(&session, "find", json!({"query": "pgbouncer", "k": 3})).await;

    session.cancel().await.expect("close the session");

    let recorded = rows(&data_dir);
    let commands: Vec<&str> = recorded.iter().map(|r| r.command.as_str()).collect();
    assert!(
        commands.contains(&"save") && commands.contains(&"find"),
        "both tool calls are on record: {commands:?}"
    );
    assert!(
        recorded
            .iter()
            .all(|r| r.source == "mcp" && r.actor.as_deref() == Some("agent-x/9.9")),
        "every row from this session names the same host"
    );
}
