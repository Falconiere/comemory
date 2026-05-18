//! `qwick gc` — purge entries in `memories/.trash/` older than 30 days.
//!
//! Uses filesystem mtime (`std::fs::Metadata::modified`) rather than a
//! frontmatter-derived cutoff timestamp; this keeps gc working even on
//! trash entries whose frontmatter is unparsable, and avoids re-reading
//! every file. The 30-day retention is intentionally fixed in v1 — the
//! plan's `cutoff` value is computed but not consulted, to keep the door
//! open for swapping in a frontmatter-aware sweep later.

use std::io::Write as _;
use std::path::PathBuf;

use time::{Duration, OffsetDateTime};

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::output::json;
use crate::prelude::*;

const RETENTION_DAYS: i64 = 30;

/// Remove every file in the trash directory whose mtime is older than the
/// retention window (`RETENTION_DAYS` days). Missing trash directory is a
/// no-op. Reports the count of files removed.
pub async fn run(json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let _cutoff = OffsetDateTime::now_utc() - Duration::days(RETENTION_DAYS);
    let mut removed = 0u64;

    if let Ok(rd) = std::fs::read_dir(paths.trash_dir()) {
        for entry in rd.flatten() {
            let too_old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|d| d > std::time::Duration::from_secs((RETENTION_DAYS as u64) * 86_400))
                .unwrap_or(false);
            if too_old && std::fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
    }

    if json_flag {
        json::write(&serde_json::json!({ "removed": removed }))?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "gc removed {removed} trashed memories")?;
    }
    Ok(())
}
