//! Tests for the embed-command shell-out ([`embed_query`]).
//!
//! Uses real shell commands (no mocks): a `printf` that emits a payload, a
//! `cat` that round-trips stdin, and failing commands that must surface as
//! errors rather than panics.

use comemory::tui::embed::embed_query;

#[test]
fn printf_command_yields_vector() {
    let cmd = r#"printf '{"embedding":[0.1,0.2,0.3]}'"#;
    let v = embed_query(cmd, "any query").expect("embed_query");
    assert_eq!(v, vec![0.1_f32, 0.2, 0.3]);
}

#[test]
fn command_reads_query_from_stdin() {
    // `cat` echoes stdin straight to stdout, proving the query is piped in.
    let payload = r#"{"embedding":[1.0,2.0]}"#;
    let v = embed_query("cat", payload).expect("embed via stdin");
    assert_eq!(v, vec![1.0_f32, 2.0]);
}

#[test]
fn nonzero_exit_is_error() {
    let err = embed_query("exit 3", "q").expect_err("nonzero exit should fail");
    let msg = format!("{err}");
    assert!(
        msg.contains("embed-cmd"),
        "expected embed-cmd error, got: {msg}"
    );
}

#[test]
fn garbage_output_is_error() {
    // Not JSON → parse error. Any Error is acceptable; the point is no panic.
    let err = embed_query("printf 'not json'", "q");
    assert!(err.is_err(), "garbage output must be an error");
}
