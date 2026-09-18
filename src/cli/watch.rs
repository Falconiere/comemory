//! `comemory watch` — arguments, launch and reporting for the workspace
//! channel. The channel policy itself — ticket, nudges, reconnect jitter — is
//! [`crate::domains::sync::watch`].
//!
//! Every platform call except the socket goes through
//! [`crate::cli::off_runtime::off_runtime`]: `domains::sync::client` and `run_pull` are
//! `reqwest::blocking`, which panics on drop inside this command's runtime.
//! That escape hatch belongs to this adapter, so it is handed to the service
//! rather than reached for from inside it.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::load_config;
use crate::cli::off_runtime::off_runtime;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::sync::AuthFile;
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

    watch::follow(&paths, &cfg, &auth, a.once, &CliOffRuntime, &mut |event| {
        report(json_flag, event)
    })
    .await
}

/// One line per event, JSON or TTY.
fn report(json_flag: bool, event: WatchEvent) -> Result<()> {
    if json_flag {
        let (name, pulled) = match event {
            WatchEvent::Connected => ("connected", 0),
            WatchEvent::Pulled(pulled) => ("pulled", pulled),
        };
        return crate::output::json::write(&serde_json::json!({
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
