#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Real-subprocess tests for the bounded runner.
//!
//! Every child here is a real `/bin/sh` process over real pipes — no mock, no
//! in-process stand-in. The payloads deliberately exceed both pipe buffers
//! (64 KiB on Linux, 16 KiB on macOS) so the concurrency is exercised rather
//! than hidden by a buffer that swallows the whole write.

use std::ffi::OsString;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use comemory::utilities::process_runner::{
    DEFAULT_PROCESS_TIMEOUT, ProcessFailure, ProcessLimits, ProcessRunner,
};

/// A payload far larger than any pipe buffer, so a partial write blocks.
const BIG: usize = 4 << 20;

/// Build a runner over `sh -c <script>` with the given extra arguments.
///
/// `sh` is the *program*; the script is an argument. Nothing is ever
/// concatenated into a command line by the runner itself.
fn sh(script: &str, extra: &[&str]) -> ProcessRunner {
    let mut args = vec![
        OsString::from("-c"),
        OsString::from(script),
        OsString::from("sh"),
    ];
    args.extend(extra.iter().map(OsString::from));
    ProcessRunner::new("/bin/sh", args)
}

/// Count this process's own zombie children, read out of the OS process table.
///
/// This is the independent evidence that the runner really reaped what it
/// killed: it observes the operating system, not the runner's control flow.
fn own_zombies() -> usize {
    let me = std::process::id().to_string();
    let out = std::process::Command::new("ps")
        .args(["-e", "-o", "ppid=", "-o", "stat="])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let ppid = parts.next()?;
            let stat = parts.next()?;
            (ppid == me && stat.starts_with('Z')).then_some(())
        })
        .count()
}

#[test]
fn round_trips_stdin_to_stdout() {
    let out = sh("cat", &[]).run(b"hello protocol").expect("run");
    assert_eq!(out.stdout, b"hello protocol");
    assert!(out.status.success());
    assert!(!out.input_truncated);
    assert!(out.stderr.is_empty());
}

#[test]
fn nonzero_exit_is_a_completed_run() {
    // Reading the status is the caller's policy, so the run itself succeeds.
    let out = sh("printf partial; exit 3", &[]).run(b"").expect("run");
    assert_eq!(out.stdout, b"partial");
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn missing_program_is_a_spawn_failure() {
    let runner = ProcessRunner::new("/nonexistent/reranker-binary", vec![]);
    match runner.run(b"{}").map_err(|e| e.failure) {
        Err(ProcessFailure::Spawn(message)) => assert!(!message.is_empty()),
        other => panic!("expected Spawn, got {other:?}"),
    }
}

#[test]
fn input_over_the_cap_never_spawns() {
    let dir = tempdir("input-cap");
    let sentinel = dir.path().join("spawned");
    let runner = sh(r#"touch "$1""#, &[&sentinel.to_string_lossy()]).with_limits(ProcessLimits {
        max_input_bytes: 8,
        ..ProcessLimits::default()
    });
    match runner.run(b"far too many bytes").map_err(|e| e.failure) {
        Err(ProcessFailure::InputTooLarge { bytes, max }) => {
            assert_eq!(bytes, 18);
            assert_eq!(max, 8);
        }
        other => panic!("expected InputTooLarge, got {other:?}"),
    }
    assert!(!sentinel.exists(), "the refusal must precede the spawn");
}

#[test]
fn child_that_writes_before_reading_does_not_deadlock() {
    // The case that deadlocks embed.rs's write-then-read sequencing: the child
    // fills the stdout pipe before draining a stdin far larger than its buffer.
    let dir = tempdir("write-first");
    let payload = dir.path().join("payload");
    std::fs::write(&payload, "o".repeat(512 * 1024)).unwrap();
    let runner = sh(
        r#"cat "$1"; cat > /dev/null"#,
        &[&payload.to_string_lossy()],
    )
    .with_timeout(Duration::from_secs(20));

    let started = Instant::now();
    let out = runner.run(&vec![b'q'; BIG]).expect("concurrent pipes");
    assert_eq!(out.stdout.len(), 512 * 1024);
    assert!(out.status.success());
    assert!(!out.input_truncated, "the child drained all of stdin");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn child_that_never_reads_stdin_still_returns_its_output() {
    // `exec <&-` closes fd 0 before printing, so the 4 MiB write hits EPIPE.
    let out = sh("exec <&-; printf hello", &[])
        .run(&vec![b'q'; BIG])
        .expect("EPIPE on stdin is the child's choice, not a failure");
    assert_eq!(out.stdout, b"hello");
    assert!(out.input_truncated, "the write could not complete");
    assert!(out.status.success());
}

#[test]
fn child_that_never_exits_times_out_and_is_reaped() {
    let started = Instant::now();
    let err = sh("sleep 5", &[])
        .with_timeout(Duration::from_millis(300))
        .run(b"")
        .expect_err("a child that outlives the budget must fail")
        .failure;
    assert!(matches!(err, ProcessFailure::TimedOut { .. }), "{err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        own_zombies(),
        0,
        "the killed child must also be reaped: nextest gives this test its own \
         process, so any zombie here is ours"
    );
}

#[test]
fn child_that_closes_stdout_but_stays_alive_times_out() {
    // stdout reaches EOF immediately; the process does not exit. Only a budget
    // that also covers the exit can end this run.
    let started = Instant::now();
    let err = sh("exec >&-; sleep 5", &[])
        .with_timeout(Duration::from_millis(300))
        .run(b"")
        .expect_err("an alive child with a closed stdout must fail")
        .failure;
    assert!(matches!(err, ProcessFailure::TimedOut { .. }), "{err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(own_zombies(), 0, "killed and reaped, leaving no zombie");
}

#[test]
fn child_whose_descendant_holds_the_pipe_times_out() {
    // The shell exits 0 at once, but the backgrounded `sleep` inherited stdout,
    // so EOF never arrives and the payload can never be known to be complete.
    let started = Instant::now();
    let err = sh("sleep 3 & exit 0", &[])
        .with_timeout(Duration::from_millis(300))
        .run(b"")
        .expect_err("an unterminated stdout must not be treated as complete")
        .failure;
    assert!(matches!(err, ProcessFailure::TimedOut { .. }), "{err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(own_zombies(), 0, "the direct child is ours to reap");
}

#[test]
fn stdout_over_the_cap_fails_fast() {
    let started = Instant::now();
    let err = sh("yes x | head -c 200000", &[])
        .with_timeout(Duration::from_secs(20))
        .with_limits(ProcessLimits {
            max_stdout_bytes: 1024,
            ..ProcessLimits::default()
        })
        .run(b"")
        .expect_err("an oversized payload must be refused")
        .failure;
    match err {
        ProcessFailure::StdoutTooLarge { max } => assert_eq!(max, 1024),
        other => panic!("expected StdoutTooLarge, got {other:?}"),
    }
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "overflow must not wait for the deadline: {:?}",
        started.elapsed()
    );
}

#[test]
fn stdout_exactly_at_the_cap_succeeds_and_one_byte_over_fails() {
    // The cap is "more than this is refused", not "this much is refused", so
    // the two sides of the boundary must land on opposite verdicts.
    let at_cap = sh("yes x | head -c 1024", &[])
        .with_limits(ProcessLimits {
            max_stdout_bytes: 1024,
            ..ProcessLimits::default()
        })
        .run(b"")
        .expect("a payload exactly at the cap is within it");
    assert_eq!(at_cap.stdout.len(), 1024);
    assert!(at_cap.status.success());

    let over_cap = sh("yes x | head -c 1025", &[])
        .with_limits(ProcessLimits {
            max_stdout_bytes: 1024,
            ..ProcessLimits::default()
        })
        .run(b"")
        .expect_err("one byte over the cap must be refused")
        .failure;
    assert!(
        matches!(over_cap, ProcessFailure::StdoutTooLarge { max: 1024 }),
        "{over_cap:?}"
    );
}

#[test]
fn oversized_stderr_is_drained_and_capped_without_stalling() {
    // A child writing 200 KB of diagnostics would block on a full stderr pipe
    // if the reader stopped at the cap. It must keep draining and discard.
    let out = sh("yes e | head -c 200000 >&2; printf done", &[])
        .with_timeout(Duration::from_secs(20))
        .with_limits(ProcessLimits {
            max_stderr_bytes: 64,
            ..ProcessLimits::default()
        })
        .run(b"")
        .expect("a chatty child is not a failing child");
    assert_eq!(out.stdout, b"done");
    assert_eq!(out.stderr.len(), 64, "retained excerpt is capped");
    assert!(out.status.success());
}

#[test]
fn one_runner_serves_concurrent_runs() {
    let runner = sh("cat", &[]);
    std::thread::scope(|scope| {
        let a = scope.spawn(|| runner.run(b"first").expect("a"));
        let b = scope.spawn(|| runner.run(b"second").expect("b"));
        assert_eq!(a.join().unwrap().stdout, b"first");
        assert_eq!(b.join().unwrap().stdout, b"second");
    });
}

/// A temporary directory that cleans itself up when the test ends.
fn tempdir(tag: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(&format!("comemory-{tag}-"))
        .tempdir()
        .expect("tempdir")
}

#[test]
fn an_enormous_budget_does_not_overflow_the_deadline() {
    // `Instant + Duration` panics on overflow and the budget is a public
    // input, so `Duration::MAX` must mean "effectively unbounded", not abort.
    let out = sh("printf ok", &[])
        .with_timeout(Duration::MAX)
        .run(b"")
        .expect("an enormous budget is still a budget");
    assert_eq!(out.stdout, b"ok");
}

#[test]
fn a_failure_carries_the_stderr_drained_before_it() {
    // A scorer that narrates its start-up and then hangs leaves no other
    // clue, so the excerpt has to travel with the failure, not just success.
    let err = sh("printf 'loading weights...' >&2; sleep 5", &[])
        .with_timeout(Duration::from_millis(300))
        .run(b"")
        .expect_err("the child outlives its budget");
    assert!(matches!(err.failure, ProcessFailure::TimedOut { .. }));
    assert_eq!(err.stderr, b"loading weights...");
}

#[test]
fn the_published_process_defaults_are_what_the_design_document_states() {
    assert_eq!(DEFAULT_PROCESS_TIMEOUT, Duration::from_secs(10));
    let limits = ProcessLimits::default();
    assert_eq!(limits.max_input_bytes, 8 * 1024 * 1024);
    assert_eq!(limits.max_stdout_bytes, 8 * 1024 * 1024);
    assert_eq!(limits.max_stderr_bytes, 16 * 1024);
}
