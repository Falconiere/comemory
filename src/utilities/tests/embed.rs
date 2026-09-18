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
    // The child closes its stdin explicitly (`exec <&-`, POSIX sh) before
    // printing, and the query is larger than any pipe buffer (1 MiB against
    // 64 KiB on Linux, 16 KiB on macOS), so the parent's write cannot
    // complete and fails with EPIPE deterministically. That is the
    // command's choice, not an error — the vector it printed must still
    // come back.
    let cmd = r#"exec <&-; printf '{"embedding":[4.0,5.0]}'"#;
    let big_query = "q".repeat(1 << 20);
    let v = embed_query(cmd, &big_query).expect("EPIPE on stdin must be tolerated");
    assert_eq!(v, vec![4.0_f32, 5.0]);
}

#[test]
fn command_that_answers_then_refuses_to_exit_times_out() {
    // The child drains stdin, prints a perfectly valid payload, closes stdout
    // (so the read reaches EOF) and then refuses to exit. Before #211 the
    // budget stopped before the final `wait`, so this pinned the caller for
    // the child's whole lifetime while looking bounded. The budget now spans
    // the exit, so it fails promptly instead.
    let started = Instant::now();
    let err = embed_query_with_timeout(
        r#"cat > /dev/null; printf '{"embedding":[1.0]}'; exec >&-; sleep 5"#,
        "q",
        Duration::from_millis(200),
    )
    .expect_err("a child that never exits must not succeed inside the budget");
    assert!(
        format!("{err}").contains("embed-cmd timed out"),
        "expected the timeout wording, got: {err}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the budget must cover the exit: {:?}",
        started.elapsed()
    );
}

#[test]
fn command_that_writes_before_reading_does_not_deadlock() {
    // A child that fills its stdout pipe before draining a stdin larger than
    // the pipe buffer deadlocked the old write-then-read sequencing outright:
    // neither side could move and no timer was running. Concurrent pipe
    // servicing makes it an ordinary round trip.
    let started = Instant::now();
    let big_query = "q".repeat(1 << 21);
    let v = embed_query_with_timeout(
        r#"printf '{"embedding":[2.0,3.0]}'; cat > /dev/null"#,
        &big_query,
        Duration::from_secs(20),
    )
    .expect("concurrent stdin/stdout must not deadlock");
    assert_eq!(v, vec![2.0_f32, 3.0]);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn nonzero_exit_reports_the_exit_status_wording() {
    // `nonzero_exit_is_error` above asserts only the `embed-cmd` prefix every
    // failure carries, so it would pass for a timeout or a spawn error too.
    // This pins the specific wording `comemory doctor` surfaces.
    let err = embed_query("printf '{\"embedding\":[1.0]}'; exit 7", "q")
        .expect_err("a non-zero exit must fail even with a valid payload");
    let msg = format!("{err}");
    assert!(
        msg.contains("embed-cmd exited with"),
        "expected the exit-status wording, got: {msg}"
    );
    assert!(msg.contains('7'), "the status itself must be named: {msg}");
}
