//! Parsing for the temporal flag values (`--since`, `--until`,
//! `--as-of`) shared by `search` and `context`.
//!
//! Two shapes are accepted: a full RFC3339 / ISO-8601 timestamp, taken
//! verbatim, or a bare `YYYY-MM-DD` date, which expands to the requested
//! edge of that UTC day ([`DayEdge`]) so `--until 2026-03-10` includes
//! everything created that day.

use time::format_description::well_known::{Iso8601, Rfc3339};
use time::macros::{format_description, time};
use time::{Date, OffsetDateTime};

use crate::prelude::*;

/// Which edge of a bare `YYYY-MM-DD` day the value expands to. Full
/// timestamps ignore it — they already name an instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayEdge {
    /// `00:00:00Z` — the lower bound of the day (`--since`).
    Start,
    /// `23:59:59.999999999Z` — the upper bound of the day (`--until`,
    /// `--as-of`), at SQLite-beating nanosecond precision so no timestamp
    /// stored that day can fall outside it.
    End,
}

/// Parse a user-supplied `--since` / `--until` / `--as-of` value into an
/// instant, expanding a bare date to `edge` of that UTC day.
///
/// Returns [`Error::Usage`] (exit `EX_USAGE`) naming both accepted
/// formats; the caller prefixes the flag name.
pub fn parse_when(s: &str, edge: DayEdge) -> Result<OffsetDateTime> {
    let raw = s.trim();
    if let Ok(ts) = OffsetDateTime::parse(raw, &Rfc3339) {
        return Ok(ts);
    }
    if let Ok(ts) = OffsetDateTime::parse(raw, &Iso8601::DEFAULT) {
        return Ok(ts);
    }
    let date = Date::parse(raw, format_description!("[year]-[month]-[day]")).map_err(|_| {
        Error::Usage(format!(
            "invalid time value `{s}` (expected an RFC3339 timestamp or YYYY-MM-DD)"
        ))
    })?;
    Ok(date.with_time(day_edge_time(edge)).assume_utc())
}

/// The wall-clock time a bare date expands to for each [`DayEdge`].
fn day_edge_time(edge: DayEdge) -> time::Time {
    match edge {
        DayEdge::Start => time!(00:00:00),
        DayEdge::End => time!(23:59:59.999999999),
    }
}
