#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! Shared real-binary `comemory mcp` runner for the stdio journey
//! (`tests/cli_scenario_mcp.rs`) — the MCP twin of `serve_bin.rs`.
//!
//! Spawns the real binary as a child process under a throwaway
//! `COMEMORY_DATA_DIR` (`<temp>/.comemory`, the production layout, same
//! convention as `serve_bin.rs`), speaks to it with the REAL rmcp client over
//! the child's stdin/stdout, and kills the child on drop. Nothing here frames
//! JSON-RPC by hand: a hand-rolled framing would prove the test's framing
//! works, not the server's.
//!
//! [`McpHome::attach`] opens a SECOND server on an EXISTING data dir from a
//! different working directory — the AC-3 shape, where a memory saved from a
//! linked worktree must be visible to a session started in the main one.
//! `dead_code` is allowed because each journey uses its own subset.

use std::path::{Path, PathBuf};
use std::time::Duration;

use assert_cmd::cargo::cargo_bin;
use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;
use tempfile::TempDir;

/// How long one handshake or tool call may take. Generous on purpose: the
/// first execution of a freshly built binary on macOS pays a one-off
/// Gatekeeper assessment measured in seconds.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// A running `comemory mcp` child and the client talking to it.
pub struct McpHome {
    /// `Some` only for the session that OWNS the temp dir; an `attach`ed
    /// second session borrows the same directory and must not delete it.
    root: Option<TempDir>,
    data_dir: PathBuf,
    client: RunningService<RoleClient, ()>,
}

impl McpHome {
    /// Spawn `comemory mcp <extra…>` on a fresh temp data dir, with the child
    /// running in `cwd` — which is what the session's default repo scope is
    /// derived from.
    pub async fn spawn(cwd: &Path, extra_args: &[&str]) -> Self {
        let root = TempDir::new().expect("tempdir");
        let data_dir = root.path().join(".comemory");
        let client = connect(&data_dir, cwd, extra_args).await;
        Self {
            root: Some(root),
            data_dir,
            client,
        }
    }

    /// A SECOND server over this session's data dir, started in `cwd`. The
    /// temp dir stays owned by `self`, so the returned handle must not
    /// outlive it.
    pub async fn attach(&self, cwd: &Path, extra_args: &[&str]) -> Self {
        Self {
            root: None,
            data_dir: self.data_dir.clone(),
            client: connect(&self.data_dir, cwd, extra_args).await,
        }
    }

    /// `<temp>/.comemory` — the server's `COMEMORY_DATA_DIR`.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// `<temp>` — where a journey drops git fixtures, beside the data dir.
    pub fn workspace(&self) -> &Path {
        self.root
            .as_ref()
            .expect("workspace() is only meaningful on the owning session")
            .path()
    }

    /// The server's `instructions`, from the `initialize` handshake.
    pub fn instructions(&self) -> String {
        self.client
            .peer_info()
            .expect("the handshake completed, so peer info is present")
            .instructions
            .clone()
            .expect("comemory mcp always advertises instructions")
    }

    /// `tools/list`, as the host sees it.
    pub async fn list_tools(&self) -> Vec<Tool> {
        with_timeout("tools/list", self.client.list_all_tools())
            .await
            .expect("tools/list")
    }

    /// `tools/call` with a JSON object of arguments. Returns the result even
    /// when it is a tool-level error — judging that is the caller's job.
    pub async fn call(&self, name: &str, arguments: Value) -> CallToolResult {
        let arguments = match arguments {
            Value::Object(map) => map,
            other => panic!("tool arguments must be a JSON object, got {other}"),
        };
        let params = CallToolRequestParams::new(name.to_string()).with_arguments(arguments);
        with_timeout(name, self.client.call_tool(params))
            .await
            .unwrap_or_else(|e| panic!("tools/call {name} failed at the protocol level: {e}"))
    }

    /// A successful call's `structured_content`, panicking with the tool's
    /// own error body when the tool failed.
    pub async fn data(&self, name: &str, arguments: Value) -> Value {
        let result = self.call(name, arguments).await;
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

    /// The `{code, message}` object of a tool-level error, asserting the call
    /// really did fail at the tool level (`is_error`), not at the protocol
    /// one.
    pub async fn error(&self, name: &str, arguments: Value) -> Value {
        let result = self.call(name, arguments).await;
        assert_eq!(
            result.is_error,
            Some(true),
            "tool `{name}` was expected to fail: {:?}",
            result.structured_content
        );
        result
            .structured_content
            .unwrap_or_else(|| panic!("tool `{name}` error carried no structured content"))
    }

    /// Close the session and reap the child. Called by the journey when a
    /// second server must see the first one's writes settled.
    pub async fn cancel(self) {
        self.client
            .cancel()
            .await
            .expect("cancel the mcp child session");
    }
}

/// Spawn one child and run the rmcp `initialize` handshake against it.
async fn connect(
    data_dir: &Path,
    cwd: &Path,
    extra_args: &[&str],
) -> RunningService<RoleClient, ()> {
    let mut cmd = tokio::process::Command::new(cargo_bin("comemory"));
    cmd.arg("mcp")
        .args(extra_args)
        .env("COMEMORY_DATA_DIR", data_dir)
        .current_dir(cwd);
    // stderr is inherited (rmcp's builder default), so a server that dies
    // during startup prints its reason into the test output instead of
    // failing as a silent EOF.
    let transport = TokioChildProcess::new(cmd).expect("spawn comemory mcp");
    with_timeout("initialize", ().serve(transport))
        .await
        .expect("initialize handshake")
}

/// Bound one protocol round-trip so a hung server fails the test with a
/// named timeout instead of stalling the suite.
async fn with_timeout<T, E: std::fmt::Display, F: Future<Output = Result<T, E>>>(
    what: &str,
    f: F,
) -> Result<T, E> {
    tokio::time::timeout(CALL_TIMEOUT, f)
        .await
        .unwrap_or_else(|elapsed| panic!("`{what}` did not answer: {elapsed}"))
}
