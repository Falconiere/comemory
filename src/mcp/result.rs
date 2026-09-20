//! Command outcomes and crate errors shaped into MCP results.
//!
//! Two failure channels, split by [`Class`]: everything the caller can act on
//! becomes a TOOL-level `structured_error` carrying the same code word
//! `serve::envelope` puts in an HTTP envelope, so an agent sees the message
//! and can retry or rephrase. Only [`Class::Internal`] becomes a protocol
//! [`ErrorData`], which hosts render opaquely — its message is logged to
//! stderr, never echoed.

use rmcp::model::{CallToolResult, ErrorData};
use serde::Serialize;
use serde_json::json;

use crate::prelude::*;
use crate::utilities::error_code::{Class, classify};

/// Code word for a mutating tool refused by a `--read-only` session.
pub const READ_ONLY_CODE: &str = "read_only";

/// Code word for a write that resolved to no repo scope.
pub const REPO_REQUIRED_CODE: &str = "repo_required";

/// The message every `repo_required` refusal carries.
const REPO_REQUIRED_MESSAGE: &str =
    "no repo scope: pass `repo`, or start the server with --repo or inside a git work tree";

/// The prefix a read-only refusal carries through `Error::Forbidden`, and the
/// only string [`into_tool_result`] matches on.
///
/// `exec::run` refuses an `Access::Write` before it builds its closure, but
/// it returns the same `Result<T>` a read does so every tool body stays one line. The
/// marker is how that one `Err` keeps the `read_only` code word instead of
/// collapsing into the generic `forbidden` row — built by [`read_only_error`]
/// and recognised by the `starts_with` arm in [`into_tool_result`], both here.
const READ_ONLY_PREFIX: &str = "server is read-only: refusing the mutating tool ";

/// A tool-level error result: `{"code": …, "message": …}` as structured
/// content, with `isError` set. The request succeeded; the tool did not.
fn tool_error(code: &str, message: &str) -> CallToolResult {
    CallToolResult::structured_error(json!({ "code": code, "message": message }))
}

/// The `read_only` refusal for `tool`, ready to return from a tool body.
pub fn read_only(tool: &str) -> CallToolResult {
    tool_error(READ_ONLY_CODE, &read_only_message(tool))
}

/// The `repo_required` refusal: a write whose repo is still empty after
/// [`crate::mcp::scope::resolve`] has had its say.
pub fn repo_required() -> CallToolResult {
    tool_error(REPO_REQUIRED_CODE, REPO_REQUIRED_MESSAGE)
}

/// The error `exec::run` returns instead of running a write closure on a
/// `--read-only` session. [`into_tool_result`] maps it back to exactly what
/// [`read_only`] builds.
pub fn read_only_error(tool: &str) -> Error {
    Error::Forbidden(read_only_message(tool))
}

/// The one message both the refusal error and [`read_only`] carry.
fn read_only_message(tool: &str) -> String {
    format!("{READ_ONLY_PREFIX}`{tool}`")
}

/// Shape a core's outcome into an MCP tool result.
///
/// `Ok` becomes `CallToolResult::structured`, which also carries the same
/// JSON in a `text` block for hosts that predate structured content. `Err`
/// becomes a tool-level error for every class but [`Class::Internal`], which
/// becomes a protocol error carrying only the code word.
pub fn into_tool_result<T: Serialize>(outcome: Result<T>) -> Result<CallToolResult, ErrorData> {
    match outcome {
        Ok(value) => match serde_json::to_value(&value) {
            Ok(payload) => Ok(CallToolResult::structured(payload)),
            // The response type is ours, so a failed serialization is a bug
            // here, not a caller error: log it and answer opaquely.
            Err(e) => {
                tracing::warn!(error = %e, "mcp: tool response failed to serialize");
                Err(ErrorData::internal_error("internal", None))
            }
        },
        // The refusal `read_only_error` minted, recognised by its marker and
        // answered with byte-identical content to `read_only(tool)`.
        Err(Error::Forbidden(message)) if message.starts_with(READ_ONLY_PREFIX) => {
            Ok(tool_error(READ_ONLY_CODE, &message))
        }
        Err(e) => {
            let (code, class) = classify(&e);
            if class == Class::Internal {
                tracing::warn!(code, error = %e, "mcp: tool failed");
                Err(ErrorData::internal_error(code, None))
            } else {
                Ok(tool_error(code, &e.to_string()))
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/result.rs"]
mod tests;
