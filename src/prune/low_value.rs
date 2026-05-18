//! Low-value detection: memories that are quality-poor, unused, and stale.
//!
//! A memory qualifies as low-value when *all three* hold:
//! - `quality < below_quality`
//! - feedback `used_count == 0`
//! - `created` is older than `unused_since_days` ago
//!
//! The detector opens a fresh [`StatsDb`] per call (as required by the
//! task spec) so it can read the feedback counters without owning a
//! long-lived connection. Detection is read-only; the CLI surface is what
//! actually soft-deletes.

use time::{Duration, OffsetDateTime};

use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::prelude::*;
use crate::stats::feedback::Feedback;
use crate::stats::sqlite::StatsDb;

/// Return the ids of memories that match every low-value criterion.
///
/// `below_quality` is a strict upper bound on `frontmatter.quality`;
/// `unused_since_days` is the minimum age (in days) since `created` for
/// a memory to qualify.
pub fn detect(paths: &Paths, below_quality: u8, unused_since_days: u32) -> Result<Vec<String>> {
    let store = MemoryStore::new(paths.clone());
    let mems = store.list()?;
    let mut db = StatsDb::open(paths.stats_db())?;
    let cutoff = OffsetDateTime::now_utc() - Duration::days(i64::from(unused_since_days));

    let mut out = Vec::new();
    for m in mems {
        if m.frontmatter.quality >= below_quality {
            continue;
        }
        if m.frontmatter.created > cutoff {
            continue;
        }
        let (used, _) = Feedback::new(&mut db).counts(&m.frontmatter.id)?;
        if used > 0 {
            continue;
        }
        out.push(m.frontmatter.id);
    }
    Ok(out)
}
