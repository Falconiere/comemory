#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/mcp/result.rs`: the success shape, the tool-level
//! error shape for every non-`Internal` class, and the protocol-error shape
//! for `Internal`.
//!
//! `store_locked` is proven against a REAL `SQLITE_BUSY` — two connections on
//! one file, the first holding `BEGIN IMMEDIATE`, the second with
//! `busy_timeout` forced to `0` — the same recipe
//! `src/utilities/tests/error_code.rs` uses, so the retryable row an agent is
//! told to retry is the one a live contended store actually produces.

use comemory::errors::Error;
use comemory::mcp::result::{self, READ_ONLY_CODE, REPO_REQUIRED_CODE, into_tool_result};
use comemory::store::connection;
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use tempfile::tempdir;

/// The `{code, message}` object of a tool-level error result.
fn tool_error_body(result: &CallToolResult) -> Value {
    assert_eq!(result.is_error, Some(true), "must be flagged as an error");
    result
        .structured_content
        .clone()
        .expect("a tool-level error carries structured content")
}

/// `into_tool_result` for an error, asserting it stayed tool-level.
fn tool_error_for(e: Error) -> Value {
    let outcome: comemory::errors::Result<Value> = Err(e);
    let result = into_tool_result(outcome).expect("must not escalate to a protocol error");
    tool_error_body(&result)
}

#[test]
fn ok_becomes_structured_content_plus_a_text_block() {
    let value = json!({"hits": [{"id": "ab12cd34"}], "query_id": "q-20260918-ab12cd34"});
    let result = into_tool_result(Ok(value.clone())).expect("ok is never a protocol error");

    assert_eq!(result.structured_content.as_ref(), Some(&value));
    assert_eq!(result.is_error, Some(false));
    assert_eq!(
        result.content.len(),
        1,
        "rmcp's `structured` already attaches the backward-compatible text block"
    );
    let text = result.content[0]
        .as_text()
        .expect("the compatibility block is text");
    assert_eq!(
        serde_json::from_str::<Value>(&text.text).expect("text block is the same JSON"),
        value
    );
}

#[test]
fn usage_and_not_found_become_tool_level_errors_with_their_code_words() {
    let body = tool_error_for(Error::Usage("pass --query or a positional query".into()));
    assert_eq!(body["code"], json!("usage"));
    assert_eq!(
        body["message"],
        json!("pass --query or a positional query"),
        "the caller sees the real message, not a placeholder"
    );

    let body = tool_error_for(Error::NotFound("ab12cd34".into()));
    assert_eq!(body["code"], json!("not_found"));
    assert_eq!(body["message"], json!("memory not found: ab12cd34"));
}

#[test]
fn an_internal_error_becomes_a_protocol_error_carrying_only_the_code_word() {
    let outcome: comemory::errors::Result<Value> =
        Err(Error::Other("connection reaper panicked".into()));
    let err = into_tool_result(outcome).expect_err("Internal must escalate");
    assert_eq!(err.message, "internal");
    assert_eq!(err.data, None, "the real message stays in the stderr log");
}

#[test]
fn a_real_locked_store_surfaces_as_the_retryable_store_locked_row() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("comemory.db");
    let holder = connection::open(&db_path).expect("open holder connection");
    let contender = connection::open(&db_path).expect("open contender connection");

    holder
        .execute("BEGIN IMMEDIATE", [])
        .expect("begin immediate on holder");
    contender
        .pragma_update(None, "busy_timeout", 0_i64)
        .expect("set busy_timeout=0 on contender");
    let write_err = contender
        .execute(
            "INSERT INTO schema_meta(key, value) VALUES ('probe', '1')",
            [],
        )
        .expect_err("write must fail while the holder's transaction is live");

    let body = tool_error_for(Error::Sqlite(write_err));
    assert_eq!(
        body["code"],
        json!("store_locked"),
        "a contended store must stay tool-level so the agent can retry it"
    );

    holder.execute("ROLLBACK", []).expect("rollback holder");
}

#[test]
fn the_read_only_refusal_round_trips_to_the_read_only_code_word() {
    let body = tool_error_for(result::read_only_error("save"));
    assert_eq!(
        body["code"],
        json!(READ_ONLY_CODE),
        "a generic `forbidden` would lose the retry-never signal"
    );
    // The refusal a write access returns and the one a tool body builds directly
    // must be the same bytes.
    assert_eq!(body, tool_error_body(&result::read_only("save")));
    assert_eq!(
        body["message"].as_str().map(|m| m.contains("save")),
        Some(true),
        "the message names the refused tool"
    );
}

#[test]
fn an_unrelated_forbidden_keeps_the_generic_code_word() {
    let body = tool_error_for(Error::Forbidden("path escaped the repo root".into()));
    assert_eq!(
        body["code"],
        json!("forbidden"),
        "only the marker message maps to read_only"
    );
}

#[test]
fn repo_required_is_a_tool_level_error_naming_how_to_supply_a_scope() {
    let result = result::repo_required();
    let body = tool_error_body(&result);
    assert_eq!(body["code"], json!(REPO_REQUIRED_CODE));
    let message = body["message"].as_str().expect("a message string");
    assert!(message.contains("--repo"), "message: {message}");
}

/// A response type whose `Serialize` fails. The response types are ours, so
/// this is the "bug in our own payload" branch: the outcome is a protocol
/// error carrying only the opaque `internal` word, never a tool-level error
/// that would leak the serializer's message to the agent.
struct Unserializable;

impl serde::Serialize for Unserializable {
    fn serialize<S: serde::Serializer>(&self, _: S) -> std::result::Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("deliberately unserializable"))
    }
}

#[test]
fn a_payload_that_fails_to_serialize_is_an_opaque_protocol_error() {
    let outcome: comemory::errors::Result<Unserializable> = Ok(Unserializable);
    let err = into_tool_result(outcome).expect_err("serialization failure must not be Ok");
    assert_eq!(err.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert_eq!(err.message, "internal");
    assert!(
        err.data.is_none(),
        "the serializer's own message must not ride along as error data: {:?}",
        err.data
    );
}
