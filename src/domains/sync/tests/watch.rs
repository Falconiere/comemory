#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! Tests for [`crate::domains::sync::watch`]: the reconnect schedule, which
//! needs no socket, and the frame loop, which gets a real one.

use std::time::Duration;

use comemory::domains::sync::watch::backoff_delay;

#[test]
fn backoff_grows_to_a_thirty_second_ceiling_and_never_drops_below_a_second() {
    // Full jitter: the floor is the minimum wait, the ceiling doubles per
    // attempt until it caps. A client that reconnects instantly in a loop is
    // what this schedule exists to prevent.
    assert_eq!(backoff_delay(0, 0.0), Duration::from_secs(1));
    assert_eq!(backoff_delay(0, 1.0), Duration::from_secs(1));
    assert_eq!(backoff_delay(3, 1.0), Duration::from_secs(8));
    assert_eq!(backoff_delay(20, 1.0), Duration::from_secs(30));
    assert_eq!(backoff_delay(20, 0.0), Duration::from_secs(1));
}

#[test]
fn a_fraction_outside_zero_to_one_cannot_push_the_delay_out_of_range() {
    // The fraction comes from /dev/urandom; clamping is what keeps a bad read
    // from turning into a negative or unbounded sleep.
    assert_eq!(backoff_delay(5, -1.0), Duration::from_secs(1));
    assert_eq!(backoff_delay(5, 2.0), backoff_delay(5, 1.0));
}

#[test]
fn the_midpoint_of_a_capped_window_is_halfway_between_floor_and_ceiling() {
    assert_eq!(backoff_delay(20, 0.5), Duration::from_millis(15_500));
}

// ---------------------------------------------------------------------------
// The frame loop, against a real ticket route and a real WebSocket upgrade.
// ---------------------------------------------------------------------------

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use comemory::config::{Config, Paths};
use comemory::domains::sync::AuthFile;
use comemory::domains::sync::watch::{self, WatchEvent};
use comemory::prelude::*;

use crate::test_common as common;
use crate::test_common::sync_platform_server::{SyncPlatformServer, SyncPlatformState};

/// The delivery adapter's escape hatch, standing in for `cli::off_runtime` and
/// counting what passed through it. A scoped thread is required rather than a
/// direct call: the platform client is `reqwest::blocking`, which panics on
/// drop inside a tokio runtime, so a runner that skipped the thread would
/// prove nothing about the real one.
struct RecordingOffRuntime {
    calls: AtomicUsize,
}

impl watch::OffRuntime for RecordingOffRuntime {
    fn off<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T> + Send,
        T: Send,
    {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::thread::scope(|scope| scope.spawn(f).join())
            .map_err(|_| Error::Other("platform request thread panicked".into()))?
    }
}

/// A data dir holding an org credential for `server`, and the config a pull
/// runs under.
fn logged_in(server: &SyncPlatformServer) -> (tempfile::TempDir, Paths, Config, AuthFile) {
    let secret = server.snapshot().secret;
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let auth = common::auth_fixture::seed_org_auth(
        &paths,
        &server.base,
        &secret,
        common::auth_fixture::FIXTURE_WORKSPACE,
    );
    (home, paths, Config::defaults(), auth)
}

#[tokio::test]
async fn once_pulls_on_the_first_greeting_and_reports_it_to_the_caller() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let (_home, paths, cfg, auth) = logged_in(&server);
    let off = RecordingOffRuntime {
        calls: AtomicUsize::new(0),
    };
    let seen = Mutex::new(Vec::new());

    watch::follow(&paths, &cfg, &auth, true, &off, &mut |event| {
        seen.lock().expect("events").push(event);
        Ok(())
    })
    .await
    .expect("follow --once");

    // The service renders nothing: everything it observed arrived through the
    // callback, in order, and the greeting triggered exactly one pull.
    let events = seen.into_inner().expect("events");
    assert!(
        matches!(
            events.as_slice(),
            [WatchEvent::Connected, WatchEvent::Pulled(_)]
        ),
        "unexpected event stream: {events:?}"
    );

    // Both blocking calls — the ticket mint and the triggered pull — went
    // through the injected runner rather than running on the async runtime.
    assert_eq!(off.calls.load(Ordering::SeqCst), 2);

    // And they really reached the platform.
    assert!(server.saw_path("/v1/ws/ticket"));
    assert!(server.saw_path("/v1/ws"));
}

#[tokio::test]
async fn a_second_nudge_on_one_connection_costs_exactly_one_more_empty_pull() {
    // The fixture's channel sends `hello` then `change` and closes: two nudges
    // on one connection, which is the duplicate-frame case — the second pull
    // is cursored, finds nothing left, and applies zero entries.
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let (_home, paths, cfg, auth) = logged_in(&server);
    let off = RecordingOffRuntime {
        calls: AtomicUsize::new(0),
    };
    let seen = Mutex::new(Vec::new());

    // Following rather than `--once`, bounded below the one-second backoff
    // floor so exactly one connection's frames are observed. The timeout is
    // how the loop ends: `follow` itself only stops when the caller does.
    let bounded = tokio::time::timeout(
        std::time::Duration::from_millis(800),
        watch::follow(&paths, &cfg, &auth, false, &off, &mut |event| {
            seen.lock().expect("events").push(event);
            Ok(())
        }),
    )
    .await;
    assert!(
        bounded.is_err(),
        "follow must keep watching until its caller stops it"
    );

    let events = seen.into_inner().expect("events");
    assert!(
        matches!(
            events.as_slice(),
            [
                WatchEvent::Connected,
                WatchEvent::Pulled(_),
                WatchEvent::Pulled(0)
            ]
        ),
        "expected a greeting, its pull, then one empty pull for the second \
         nudge, saw: {events:?}"
    );

    // One ticket plus one pull per nudge, every one of them off the runtime.
    assert_eq!(off.calls.load(Ordering::SeqCst), 3);
}

#[test]
fn only_hello_and_change_frames_are_nudges() {
    // The socket is advisory: a frame this build does not understand, or one
    // that is not JSON at all, is ignored rather than failing the connection.
    assert!(super::is_nudge(
        r#"{"type":"hello","workspace_id":"ws-org"}"#
    ));
    assert!(super::is_nudge(r#"{"type":"change","ops":[]}"#));
    assert!(!super::is_nudge(r#"{"type":"something-new"}"#));
    assert!(!super::is_nudge(r#"{"workspace_id":"ws-org"}"#));
    assert!(!super::is_nudge("not json at all"));
    assert!(!super::is_nudge(""));
}
