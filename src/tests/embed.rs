#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for the embed-command shell-out ([`embed_query`]).
//!
//! Uses real shell commands (no mocks): a `printf` that emits a payload, a
//! `cat` that round-trips stdin, and failing commands that must surface as
//! errors rather than panics.

use std::time::{Duration, Instant};

use comemory::embed::{embed_query, embed_query_with_timeout};

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

#[test]
fn slow_command_times_out_promptly_and_reaps() {
    // `sleep 5` outlives the 150ms bound: the timeout must fire and the child
    // be killed+reaped (not waited on for the full 5s). Asserting prompt return
    // proves we don't block on the child; the kill+wait avoids a zombie.
    let start = Instant::now();
    let err = embed_query_with_timeout("sleep 5", "q", Duration::from_millis(150));
    assert!(err.is_err(), "a command slower than the timeout must error");
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "timeout must return promptly, not wait for the child: {:?}",
        start.elapsed()
    );
}

#[test]
fn command_that_never_reads_stdin_still_yields_vector() {
    // A query larger than the pipe buffer, to a command that never reads
    // stdin: the write cannot complete before the child exits, so it fails
    // with EPIPE deterministically. That is the command's choice, not an
    // error — the vector it printed must still come back.
    let cmd = r#"printf '{"embedding":[4.0,5.0]}'"#;
    let big_query = "q".repeat(1 << 20);
    let v = embed_query(cmd, &big_query).expect("EPIPE on stdin must be tolerated");
    assert_eq!(v, vec![4.0_f32, 5.0]);
}
