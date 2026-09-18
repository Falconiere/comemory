#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The `ready` handover `comemory::serve::serve` takes since #178.
//!
//! The banner moved to `cli::serve`, so the server hands back a
//! [`comemory::serve::Ready`] once its listener is bound and before it accepts
//! anything. Two things about that callback are contract rather than
//! convenience, and neither had a test: it runs exactly once with the port
//! actually bound, and its error aborts startup instead of being swallowed —
//! a caller that cannot print the URL and token cannot reach the server
//! either, so serving on regardless would strand it.
//!
//! Both cases are wrapped in a timeout on purpose. Swallowing the callback
//! error is the exact regression under test, and its symptom is that `serve`
//! goes on to `axum::serve(...).await` and never returns: without the timeout
//! this file would HANG on a regression rather than fail, which is the worse
//! of the two outcomes. Measured — the flip was run.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use comemory::config::Config;
use comemory::config::paths::Paths;
use comemory::errors::Error;
use comemory::serve::{RootOverrides, ServeOptions};
use tempfile::TempDir;

/// Options for an ephemeral read-only session in a throwaway data dir.
fn options() -> ServeOptions {
    ServeOptions {
        repo: None,
        port: 0,
        read_only: true,
        roots: RootOverrides::new(),
        cfg: Config::defaults(),
        embed_cmd: None,
        allow_path: Vec::new(),
    }
}

#[tokio::test]
async fn a_failing_ready_callback_aborts_startup_instead_of_serving_on() {
    let home = TempDir::new().expect("home");
    let paths = Paths::new(home.path().join(".comemory"));
    let calls = AtomicUsize::new(0);

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        comemory::serve::serve(&paths, options(), &|_ready| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(Error::Other("banner writer failed".into()))
        }),
    )
    .await
    .expect("serve must return promptly, not start accepting requests");

    let err = result.expect_err("a failing ready callback must abort startup");
    // The whole message, not a substring: a `contains` here would also pass for
    // an unrelated failure that happened to mention the same words, which would
    // make this test green for the wrong reason.
    assert!(
        matches!(err, Error::Other(ref m) if m == "banner writer failed"),
        "the callback's own error must propagate verbatim and unwrapped, got: {err:?}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "ready runs exactly once, before the first request can be accepted"
    );
}

#[tokio::test]
async fn ready_reports_the_port_actually_bound_not_the_requested_zero() {
    let home = TempDir::new().expect("home");
    let paths = Paths::new(home.path().join(".comemory"));

    // Abort from inside the callback so the server never enters its accept
    // loop; what is under test is the value handed over, not the serving.
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        comemory::serve::serve(&paths, options(), &|ready| {
            assert_ne!(ready.port, 0, "--port 0 must resolve to a real bound port");
            assert_eq!(
                ready.url,
                format!("http://127.0.0.1:{}/api/v1", ready.port),
                "the URL names the API base and the bound port"
            );
            assert!(
                ready.read_only,
                "the session read-only flag is carried through"
            );
            assert_eq!(ready.token.len(), 64, "the session token is 64 hex chars");
            Err(Error::Other("stop here".into()))
        }),
    )
    .await
    .expect("serve must return promptly, not start accepting requests");

    assert!(
        result.is_err(),
        "the callback aborted, so serve returns its error"
    );
}
