//! Hold the workspace channel open and pull when it says to
//! (`2026-09-14-sync-everything-realtime-design.md`) — the pull direction's
//! answer to polling and to an installed OS unit.
//!
//! The socket carries **nudges**. Every frame — `hello` on connect, `change`
//! after someone writes — triggers the same cursored [`pull::run_pull`] a
//! manual sync runs. Nothing is read out of a frame but the fact that one
//! arrived, so a missed frame costs latency and a duplicate costs one empty
//! pull.

use std::time::Duration;

use futures::StreamExt as _;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::config::{Config, Paths};
use crate::domains::sync::{client, pull};
use crate::prelude::*;
use crate::store::connection;

use super::AuthFile;

/// Entries one triggered pull may apply.
pub const PULL_LIMIT: usize = 2000;

/// Reconnect backoff bounds.
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// What the watch service observed. The caller decides whether and how to
/// show it, so the service itself writes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchEvent {
    /// The channel accepted the ticket and is open.
    Connected,
    /// A nudge triggered a pull that applied this many entries.
    Pulled(u32),
}

/// Runs a blocking platform call with no async runtime in scope.
///
/// [`client::ws_ticket`] and [`pull::run_pull`] are `reqwest::blocking`, which
/// builds its own runtime and panics on drop when it finds another already in
/// scope. The adapter that owns the runtime owns the escape hatch, so the
/// watch service asks its caller for one rather than importing a delivery
/// module.
pub trait OffRuntime {
    /// Run `f` off the caller's runtime and return what it returned.
    ///
    /// # Errors
    /// Propagates `f`'s error.
    fn off<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T> + Send,
        T: Send;
}

/// Full-jitter backoff for attempt `n`, capped at [`BACKOFF_MAX`].
///
/// Exposed for its own test: the schedule is the part worth pinning, and it
/// needs no socket to check.
pub fn backoff_delay(attempt: u32, fraction: f64) -> Duration {
    let ceiling = BACKOFF_MIN
        .saturating_mul(1_u32 << attempt.min(16))
        .min(BACKOFF_MAX);
    let spread = ceiling.saturating_sub(BACKOFF_MIN);
    BACKOFF_MIN + spread.mul_f64(fraction.clamp(0.0, 1.0))
}

/// Follow the workspace channel until interrupted (or, with `once`, until the
/// first greeting has been acted on), reporting each event to `on_event`.
///
/// # Errors
/// [`Error::Other`] when `once` is set and no greeting ever arrived. A socket
/// that refuses or drops is not an error — it is what the reconnect loop is
/// for — and neither is a failed `on_event`: both end the current attempt and
/// are retried after a backoff, so a broken stdout cannot kill the watcher.
pub async fn follow<R: OffRuntime + Sync>(
    paths: &Paths,
    cfg: &Config,
    auth: &AuthFile,
    once: bool,
    off: &R,
    on_event: &mut dyn FnMut(WatchEvent) -> Result<()>,
) -> Result<()> {
    let mut attempt: u32 = 0;
    loop {
        match follow_once(paths, cfg, auth, once, off, on_event).await {
            Ok(true) => return Ok(()),
            Ok(false) => attempt = 0,
            Err(e) => tracing::debug!(error = %e, "workspace channel attempt failed"),
        }
        if once {
            return Err(Error::Other(
                "the workspace channel never greeted us".into(),
            ));
        }
        // Randomly jittered inside a bounded window, per client: a platform
        // restart must not bring every watcher back on the same instant. The
        // window doubles per attempt and caps at BACKOFF_MAX; `attempt` is
        // incremented after the delay is computed, so the first retry waits
        // the floor.
        let delay = backoff_delay(attempt, rand_fraction());
        attempt = attempt.saturating_add(1);
        tokio::time::sleep(delay).await;
    }
}

/// A fraction in `[0, 1)` for the backoff jitter, without an RNG dependency.
///
/// `/dev/urandom` is the preferred source and exists on both supported
/// platforms (macOS, Linux). It is read rather than assumed: on any host
/// without it the clock's sub-second remainder stands in, which is a weaker
/// spread but still a spread — unlike a constant, which would line every
/// client of a restarted platform up on the same reconnect instant.
fn rand_fraction() -> f64 {
    let mut bytes = [0_u8; 2];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_ok()
    {
        return f64::from(u16::from_le_bytes(bytes)) / f64::from(u16::MAX);
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    f64::from(nanos % 1_000) / 1_000.0
}

/// One connection's lifetime. `Ok(true)` means `once` has been satisfied.
async fn follow_once<R: OffRuntime + Sync>(
    paths: &Paths,
    cfg: &Config,
    auth: &AuthFile,
    once: bool,
    off: &R,
    on_event: &mut dyn FnMut(WatchEvent) -> Result<()>,
) -> Result<bool> {
    let api_url = auth.api_url.clone();
    let secret = auth.effective_secret();
    let ticket = off.off(move || client::ws_ticket(&api_url, &secret))?;
    let url = client::channel_url(&auth.api_url, &ticket)?;

    let (mut socket, _response) = connect_async(&url)
        .await
        .map_err(|e| Error::Other(format!("workspace channel: {e}")))?;
    on_event(WatchEvent::Connected)?;

    while let Some(frame) = socket.next().await {
        let frame = frame.map_err(|e| Error::Other(format!("workspace channel: {e}")))?;
        let Message::Text(text) = frame else {
            // Ping/pong and close are the library's business, not ours.
            continue;
        };
        if !is_nudge(&text) {
            continue;
        }
        let pulled = off.off(|| pull_now(paths, cfg, auth))?;
        on_event(WatchEvent::Pulled(pulled))?;
        if once {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether a frame is one of the two the channel sends. Anything else — a
/// malformed frame, a type this build does not know — is ignored rather than
/// treated as an error: the socket is advisory.
fn is_nudge(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    matches!(
        value.get("type").and_then(serde_json::Value::as_str),
        Some("hello" | "change")
    )
}

/// Run the same cursored pull a manual sync would.
fn pull_now(paths: &Paths, cfg: &Config, auth: &AuthFile) -> Result<u32> {
    let mut conn = connection::open(paths.db_path())?;
    let stats = pull::run_pull(paths, cfg, &mut conn, auth, PULL_LIMIT)?;
    Ok(stats.pulled)
}

#[cfg(test)]
#[path = "tests/watch.rs"]
mod tests;
