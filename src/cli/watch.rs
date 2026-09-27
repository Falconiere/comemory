//! `comemory watch` — arguments, launch and reporting for the workspace
//! channel.
//!
//! Required (#257): attaches to the resident coordinator —
//! `client::subscribe` for `pass_finished` events, `client::catch_up` for
//! `--once` — re-attaching (full-jitter backoff) on a dropped subscription
//! rather than exiting. The `COMEMORY_SYNC_DAEMON=0` harness switch keeps
//! the pre-#257 in-process follower, [`watch::follow`], instead.

use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use clap::Args as ClapArgs;

use crate::cli::load_config;
use crate::cli::off_runtime::off_runtime;
use crate::config::Config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::config::sync::{daemon_disabled, parse_duration};
use crate::domains::sync::AuthFile;
use crate::domains::sync::daemon::client;
use crate::domains::sync::daemon::control::Event;
use crate::domains::sync::watch::{self, WatchEvent};
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Follow the organization's changes until interrupted
  comemory watch

  # Pull once through the channel and exit (scripts, smoke checks)
  comemory watch --once";

/// Arguments to `comemory watch`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Pull once the channel greets, then exit instead of following.
    #[arg(long, default_value_t = false)]
    pub once: bool,
}

/// This command's runtime escape hatch, handed to the watch service so the
/// domain never names a delivery module.
struct CliOffRuntime;

impl watch::OffRuntime for CliOffRuntime {
    fn off<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T> + Send,
        T: Send,
    {
        off_runtime(f)
    }
}

/// How long opening the coordinator's event subscription may take.
const SUBSCRIBE_BOUND: Duration = Duration::from_secs(5);

/// Follow the workspace channel until interrupted (or, with `--once`, until
/// one complete catch-up has been reported).
///
/// # Errors
/// [`Error::Usage`] when the machine is not logged in. Attached to the
/// coordinator, `--once` propagates [`Error::Unavailable`] (exit 69) when the
/// subscription or catch-up never completes; following instead retries. Not
/// attached (the harness switch), a socket that refuses or drops is not an
/// error — it is what that reconnect loop is for.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    let auth = AuthFile::load(&paths)?
        .ok_or_else(|| Error::Usage("not logged in — run `comemory auth login`".into()))?;

    if daemon_disabled() {
        return watch::follow(&paths, &cfg, &auth, a.once, &CliOffRuntime, &mut |event| {
            report(json_flag, event)
        })
        .await;
    }
    off_runtime(move || {
        follow_coordinator(&paths, &cfg, a.once, &mut |event| report(json_flag, event))
    })
}

/// The coordinator-attached path: subscribe, report each `pass_finished` as
/// a `Pulled` event, and re-attach with backoff on a dropped subscription —
/// `once` instead asks for one `catch_up` and returns.
fn follow_coordinator(
    paths: &Paths,
    cfg: &Config,
    once: bool,
    on_event: &mut dyn FnMut(WatchEvent) -> Result<()>,
) -> Result<()> {
    let mut attempt: u32 = 0;
    loop {
        match attach_once(paths, cfg, once, on_event) {
            Ok(true) => return Ok(()),
            Ok(false) => attempt = 0,
            Err(e) if once => return Err(e),
            Err(e) => tracing::debug!(error = %e, "coordinator subscription attempt failed"),
        }
        let delay = watch::backoff_delay(attempt, watch::rand_fraction());
        attempt = attempt.saturating_add(1);
        std::thread::sleep(delay);
    }
}

/// One subscription's lifetime. `Ok(true)` means fully done (what `once`
/// reports); `Ok(false)` means the subscription ended and the caller should
/// re-attach.
fn attach_once(
    paths: &Paths,
    cfg: &Config,
    once: bool,
    on_event: &mut dyn FnMut(WatchEvent) -> Result<()>,
) -> Result<bool> {
    let mut sub = client::subscribe(paths, SUBSCRIBE_BOUND)?;
    on_event(WatchEvent::Connected)?;
    if once {
        let summary = client::catch_up(paths, catchup_bound(cfg)?)?;
        on_event(WatchEvent::Pulled(summary.pulled))?;
        return Ok(true);
    }
    loop {
        match sub.next(None)? {
            Some(Event::PassFinished(summary)) => on_event(WatchEvent::Pulled(summary.pulled))?,
            Some(Event::PassStarted { .. } | Event::Channel { .. }) => {}
            None => return Ok(false),
        }
    }
}

/// How long one `catch_up` may take: the exchange's own request budget plus
/// a margin, since a pass it forces can make one full round trip.
fn catchup_bound(cfg: &Config) -> Result<Duration> {
    Ok(parse_duration(&cfg.sync.request_timeout)? + Duration::from_secs(10))
}

/// One line per event, JSON or TTY.
fn report(json_flag: bool, event: WatchEvent) -> Result<()> {
    if json_flag {
        let (name, pulled) = match event {
            WatchEvent::Connected => ("connected", 0),
            WatchEvent::Pulled(pulled) => ("pulled", pulled),
        };
        return crate::cli::output::json::write(&serde_json::json!({
            "event": name,
            "pulled": pulled,
        }));
    }
    let mut out = std::io::stdout().lock();
    match event {
        WatchEvent::Connected => writeln!(out, "watching for changes (ctrl-c to stop)")?,
        WatchEvent::Pulled(pulled) => writeln!(out, "pulled {pulled} entries")?,
    }
    Ok(())
}
