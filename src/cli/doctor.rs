//! `qwick-memory doctor` — health/inventory check. Reports the data directory and
//! the number of memories currently on disk.

use std::io::Write as _;
use std::path::PathBuf;

use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::output::json;
use crate::prelude::*;

/// JSON shape emitted under `--json` and used to compute TTY output.
#[derive(Serialize)]
struct Report {
    data_dir: String,
    memories_count: usize,
}

/// Build and emit the doctor report.
pub async fn run(json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let store = MemoryStore::new(paths.clone());
    let report = Report {
        data_dir: paths.data_dir().to_string_lossy().into_owned(),
        memories_count: store.list()?.len(),
    };
    if json_flag {
        json::write(&report)?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "data_dir       : {}", report.data_dir)?;
        writeln!(out, "memories_count : {}", report.memories_count)?;
    }
    Ok(())
}
