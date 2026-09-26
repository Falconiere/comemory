//! `comemory sync --action auto` — the CLI half of `domains::sync::auto`.
//!
//! Git and agent hooks fire this from any cwd. It hands the checkout to the
//! resident coordinator over its control socket (A-5's "post-commit
//! Unix-socket notification") and returns; no in-process pass runs. Under
//! the `COMEMORY_SYNC_DAEMON=0` harness switch — where preflight never ran —
//! it keeps the old in-process pass, since there is nothing to wake. Prints
//! nothing without `--json`, because its callers' output nobody reads.

use std::path::Path;
use std::time::Duration;

use crate::cli::off_runtime::off_runtime;
use crate::cli::output::json;
use crate::config::Config;
use crate::config::paths::Paths;
use crate::config::sync::daemon_disabled;
use crate::domains::sync::auto::{self, AutoOutcome};
use crate::domains::sync::daemon::client;
use crate::domains::sync::daemon::control::Wake;
use crate::domains::sync::daemon::readiness::Trigger;
use crate::prelude::*;

/// How long the hook waits for the coordinator to accept the wake.
const WAKE_BOUND: Duration = Duration::from_secs(2);

/// Run one auto pass for `checkout` (the hook's `--path`), or for no
/// particular checkout.
pub(crate) fn run(
    paths: &Paths,
    cfg: &Config,
    checkout: Option<&Path>,
    json_flag: bool,
) -> Result<()> {
    if daemon_disabled() {
        return run_in_process(paths, cfg, checkout, json_flag);
    }
    client::wake(
        paths,
        Wake {
            checkout: checkout.map(Path::to_path_buf),
            reason: Trigger::Hook,
        },
        WAKE_BOUND,
    )?;
    if !json_flag {
        return Ok(());
    }
    let instance = match client::probe(paths, WAKE_BOUND) {
        client::Probe::Healthy(readiness) => Some(readiness.instance),
        client::Probe::NotRunning(_) | client::Probe::Stale(_) => None,
    };
    json::write(&serde_json::json!({
        "action": "auto",
        "delivered": "daemon",
        "instance": instance,
    }))
}

/// The pre-#257 path: coalesce and run the pass in this process. Only
/// reachable with the harness switch, where preflight never ensured a
/// coordinator to wake.
fn run_in_process(
    paths: &Paths,
    cfg: &Config,
    checkout: Option<&Path>,
    json_flag: bool,
) -> Result<()> {
    let outcome = off_runtime(|| auto::run_auto(paths, cfg, checkout))?;
    if !json_flag {
        return Ok(());
    }
    let AutoOutcome::Ran(stats) = outcome else {
        return json::write(&serde_json::json!({ "action": "auto", "coalesced": true }));
    };
    let mut report = serde_json::json!({
        "action": "auto",
        "coalesced": false,
        "logged_in": stats.logged_in,
        "refresh": stats.run.refresh,
    });
    if stats.logged_in {
        report["pull"] = serde_json::json!(stats.run.pull);
        report["push"] = serde_json::json!(stats.run.push);
        report["code"] = serde_json::json!(stats.run.code);
        report["exchange"] = serde_json::json!(stats.run.exchange);
        report["error"] = serde_json::json!(stats.error);
    }
    json::write(&report)
}
