//! `comemory doctor` — health/inventory check. Reports the data directory and
//! the number of memories currently on disk.

use std::io::Write as _;
use std::path::PathBuf;

use serde::Serialize;

use crate::cli::resolve_data_dir;

/// Example invocations shown at the bottom of `comemory doctor --help`.
pub const EXAMPLES: &str = "\
Examples:
  # Human-readable health report
  comemory doctor

  # JSON for monitoring or CI
  comemory doctor --json";
use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::output::json;
use crate::prelude::*;
use crate::stats::StatsDb;

/// JSON shape emitted under `--json` and used to compute TTY output.
#[derive(Serialize)]
struct Report {
    data_dir: String,
    memories_count: usize,
    index_failures: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_index_failure: Option<LastIndexFailure>,
}

/// JSON shape for the most recent recorded index failure. Emitted only when
/// at least one row exists in the `index_failures` table.
#[derive(Serialize)]
struct LastIndexFailure {
    ts: String,
    error: String,
}

/// Build and emit the doctor report.
pub async fn run(json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let store = MemoryStore::new(paths.clone());
    let (index_failures, last_index_failure) = match StatsDb::open(paths.stats_db()) {
        Ok(db) => {
            let count = db.index_failure_count().unwrap_or(0);
            let last = if count > 0 {
                db.last_index_failure()
                    .ok()
                    .flatten()
                    .map(|(ts, error)| LastIndexFailure { ts, error })
            } else {
                None
            };
            (count, last)
        }
        Err(e) => {
            tracing::warn!("doctor: stats db unavailable: {e}");
            (0, None)
        }
    };
    let report = Report {
        data_dir: paths.data_dir().to_string_lossy().into_owned(),
        memories_count: store.list()?.len(),
        index_failures,
        last_index_failure,
    };
    if json_flag {
        json::write(&report)?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "data_dir       : {}", report.data_dir)?;
        writeln!(out, "memories_count : {}", report.memories_count)?;
        match &report.last_index_failure {
            Some(last) => writeln!(
                out,
                "index_failures : {} (last: {} — {})",
                report.index_failures, last.ts, last.error
            )?,
            None => writeln!(out, "index_failures : {}", report.index_failures)?,
        }
    }
    Ok(())
}
