//! `comemory sync --action auto` — the CLI half of `domains::sync::auto`: run
//! the pass off the async runtime and, under `--json`, report it. Prints
//! nothing otherwise, because its callers are git hooks and agent hooks whose
//! output nobody reads.

use std::path::Path;

use crate::cli::off_runtime::off_runtime;
use crate::cli::output::json;
use crate::config::Config;
use crate::config::paths::Paths;
use crate::domains::sync::auto::{self, AutoOutcome};
use crate::prelude::*;

/// Run one auto pass for `checkout` (the hook's `--path`), or for no
/// particular checkout.
pub(crate) fn run(
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
        report["error"] = serde_json::json!(stats.error);
    }
    json::write(&report)
}
