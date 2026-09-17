//! `comemory watch` — hold the workspace channel open and pull when it says to
//! (`2026-09-14-sync-everything-realtime-design.md`).
//!
//! This is the pull direction's answer to the same question push answered by
//! moving into `save`: how does a second machine see a change without either
//! polling or an installed OS unit? By holding a socket in the foreground, for
//! as long as the operator wants it, and exiting when they stop it.
//!
//! The socket carries **nudges**. Every frame — `hello` on connect, `change`
//! after someone writes — triggers the same cursored `run_pull` a manual
//! `comemory sync` would run. Nothing is read out of a frame except the fact
//! that one arrived, so a missed frame costs latency and a duplicate frame
//! costs one empty pull.
//!
//! Every platform call except the socket goes through
//! [`crate::cli::off_runtime::off_runtime`]: `sync::client` and `run_pull` are
//! `reqwest::blocking`, which panics on drop inside this command's runtime.

use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use clap::Args as ClapArgs;
use futures::StreamExt as _;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::cli::load_config;
use crate::cli::off_runtime::off_runtime;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::prelude::*;
use crate::store::connection;
use crate::domains::sync::client;
use crate::domains::sync::{AuthFile, pull};

const EXAMPLES: &str = "\
Examples:
  # Follow the organization's changes until interrupted
  comemory watch

  # Pull once through the channel and exit (scripts, smoke checks)
  comemory watch --once";

/// Entries one triggered pull may apply.
const PULL_LIMIT: usize = 2000;

/// Reconnect backoff bounds.
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Arguments to `comemory watch`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Pull once the channel greets, then exit instead of following.
    #[arg(long, default_value_t = false)]
    pub once: bool,
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

/// Follow the workspace channel until interrupted (or, with `--once`, until
/// the first greeting has been acted on).
///
/// # Errors
/// [`Error::Usage`] when the machine is not logged in. A socket that refuses
/// or drops is not an error — it is what the reconnect loop is for — so the
/// only other failures are local ones (config, store).
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    let auth = AuthFile::load(&paths)?
        .ok_or_else(|| Error::Usage("not logged in — run `comemory auth login`".into()))?;

    let mut attempt: u32 = 0;
    loop {
        match follow_once(&paths, &cfg, &auth, a.once, json_flag).await {
            Ok(true) => return Ok(()),
            Ok(false) => attempt = 0,
            Err(e) => tracing::debug!(error = %e, "workspace channel attempt failed"),
        }
        if a.once {
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

/// One connection's lifetime. `Ok(true)` means `--once` has been satisfied.
async fn follow_once(
    paths: &Paths,
    cfg: &crate::config::Config,
    auth: &AuthFile,
    once: bool,
    json_flag: bool,
) -> Result<bool> {
    let api_url = auth.api_url.clone();
    let secret = auth.effective_secret();
    let ticket = off_runtime(move || client::ws_ticket(&api_url, &secret))?;
    let url = client::channel_url(&auth.api_url, &ticket)?;

    let (mut socket, _response) = connect_async(&url)
        .await
        .map_err(|e| Error::Other(format!("workspace channel: {e}")))?;
    report(json_flag, "connected", 0)?;

    while let Some(frame) = socket.next().await {
        let frame = frame.map_err(|e| Error::Other(format!("workspace channel: {e}")))?;
        let Message::Text(text) = frame else {
            // Ping/pong and close are the library's business, not ours.
            continue;
        };
        if !is_nudge(&text) {
            continue;
        }
        let pulled = pull_now(paths, cfg, auth)?;
        report(json_flag, "pulled", pulled)?;
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
fn pull_now(paths: &Paths, cfg: &crate::config::Config, auth: &AuthFile) -> Result<u32> {
    off_runtime(|| {
        let mut conn = connection::open(paths.db_path())?;
        let stats = pull::run_pull(paths, cfg, &mut conn, auth, PULL_LIMIT)?;
        Ok(stats.pulled)
    })
}

/// One line per event, JSON or TTY.
fn report(json_flag: bool, event: &str, pulled: u32) -> Result<()> {
    if json_flag {
        return crate::output::json::write(&serde_json::json!({
            "event": event,
            "pulled": pulled,
        }));
    }
    let mut out = std::io::stdout().lock();
    match event {
        "connected" => writeln!(out, "watching for changes (ctrl-c to stop)")?,
        _ => writeln!(out, "pulled {pulled} entries")?,
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/watch.rs"]
mod tests;
