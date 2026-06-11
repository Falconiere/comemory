//! Low-value memory detection driven by the same signals the rank
//! pipeline uses: activation, Beta feedback, quality, graph degree —
//! plus an independent superseded-and-forgotten rule.
//!
//! Detection is read-only; the CLI surface (`cli::prune`) is what
//! actually soft-deletes. The thresholds come from `cfg.prune`
//! (`min_activation`, `min_feedback`, `low_value_default_below_quality`)
//! and the activation decay from `cfg.rank.decay`, so prune and rerank
//! can never disagree on what "cold" means.

use rusqlite::Connection;
use time::OffsetDateTime;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::score;

/// Memories matching ALL of: activation below `cfg.prune.min_activation`,
/// Beta feedback at/below `cfg.prune.min_feedback`, quality ≤
/// `cfg.prune.low_value_default_below_quality` (inclusive), and zero
/// incoming edges — plus any memory superseded by a live one and never
/// accessed since the supersede edge was written. Returns sorted,
/// de-duplicated ids.
pub fn detect(conn: &Connection, cfg: &Config) -> Result<Vec<String>> {
    let now = OffsetDateTime::now_utc();
    let mut flagged = signal_rule(conn, cfg, now)?;
    for id in superseded_rule(conn, cfg.prune.superseded_grace_days, now)? {
        if !flagged.contains(&id) {
            flagged.push(id);
        }
    }
    flagged.sort();
    Ok(flagged)
}

/// Stale-signal rule: low quality + no incoming edges in SQL, then the
/// activation/feedback floors evaluated in Rust with the exact scoring
/// primitives the rerank stage uses.
fn signal_rule(conn: &Connection, cfg: &Config, now: OffsetDateTime) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.access_count, COALESCE(m.last_accessed, m.created_at),
                COALESCE(f.used_count, 0), COALESCE(f.irrelevant_count, 0)
           FROM memories m
           LEFT JOIN feedback f ON f.memory_id = m.id
          WHERE m.deleted_at IS NULL
            AND m.quality <= ?1
            AND NOT EXISTS (SELECT 1 FROM edges e
                             WHERE e.dst_kind = 'memory' AND e.dst_id = m.id)",
    )?;
    let rows: Vec<(String, i64, String, i64, i64)> = stmt
        .query_map([cfg.prune.low_value_default_below_quality], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let mut out = Vec::new();
    for (id, access, last, used, irrelevant) in rows {
        let days = score::days_since(&last, now);
        let act = score::activation(access.max(0) as u64, days, cfg.rank.decay);
        let beta = score::beta_feedback(used.max(0) as u64, irrelevant.max(0) as u64);
        if act < cfg.prune.min_activation && beta <= cfg.prune.min_feedback {
            out.push(id);
        }
    }
    Ok(out)
}

/// Default grace window for the superseded rule: only supersede edges
/// older than this many days count. Protects freshly-rebuilt DBs —
/// `comemory rebuild` rematerializes every edge with a rebuild-time
/// timestamp, so without a grace period every superseded memory would
/// instantly look "never accessed since the edge was written" and get
/// flagged. Operator-tunable via `cfg.prune.superseded_grace_days`
/// (env `COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS`); this constant seeds
/// that field's default.
pub(crate) const SUPERSEDED_GRACE_DAYS: u32 = 7;

/// Superseded-and-forgotten rule: a live memory superseded by another
/// *live* memory, with no access recorded since the supersede edge was
/// created — and only when the edge has aged past `grace_days`
/// (`cfg.prune.superseded_grace_days`, default
/// [`SUPERSEDED_GRACE_DAYS`]). Quality and feedback are deliberately
/// ignored here — a replaced memory nobody has touched since its
/// replacement is prune material regardless of how good it once was.
/// Self-edges (`src_id = dst_id`) are ignored as defense-in-depth; the
/// writers refuse to create them.
fn superseded_rule(conn: &Connection, grace_days: u32, now: OffsetDateTime) -> Result<Vec<String>> {
    // The `<` compares timestamps as strings. That is sound here: both
    // writer formats — `memory_row::iso_format` (Iso8601, 9-digit
    // subseconds) and the SQLite `strftime('%Y-%m-%dT%H:%M:%fZ', ...)`
    // upsert arm (3-digit) — share the fixed-width `YYYY-MM-DDTHH:MM:SS`
    // UTC prefix, so lexicographic order matches chronological order to
    // second granularity; only mixed-format sub-second ties could
    // misorder, and this rule operates at days scale.
    let cutoff =
        crate::store::memory_row::iso_format(now - time::Duration::days(i64::from(grace_days)))?;
    let mut stmt = conn.prepare(
        "SELECT old.id FROM memories old
           JOIN edges e ON e.rel = 'supersedes'
                       AND e.src_kind = 'memory'
                       AND e.dst_kind = 'memory' AND e.dst_id = old.id
                       AND e.src_id <> e.dst_id
           JOIN memories newer ON newer.id = e.src_id AND newer.deleted_at IS NULL
          WHERE old.deleted_at IS NULL
            AND COALESCE(old.last_accessed, old.created_at) < e.created_at
            AND e.created_at < ?1",
    )?;
    let ids = stmt
        .query_map([cutoff], |r| r.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(ids)
}
