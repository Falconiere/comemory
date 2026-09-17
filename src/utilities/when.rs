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
use crate::retrieval::scope::TimeScope;
use crate::store::memory_row;

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
            "invalid time value `{raw}` (expected an RFC3339 timestamp or YYYY-MM-DD)"
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

/// Build the run's [`TimeScope`] from the raw `--since` / `--until` /
/// `--as-of` values. Shared by `comemory search` and `comemory context` so
/// the two commands cannot drift on parsing, normalization, or validation.
///
/// `--until` and `--as-of` are mutually exclusive (clap enforces it) and both
/// land in [`TimeScope::cutoff`]; only `--as-of` sets [`TimeScope::as_of`],
/// which additionally scopes the supersede penalty. Both bounds are
/// normalized through [`memory_row::iso_format`] — the single writer format —
/// so the store layer compares like against like.
///
/// Returns [`Error::Usage`] (exit `EX_USAGE`) naming the offending flag when
/// a value is unparsable or when `--since` is later than the cutoff.
pub fn scope_from_flags(
    since: Option<&str>,
    until: Option<&str>,
    as_of: Option<&str>,
) -> Result<TimeScope> {
    // `--as-of` wins the cutoff slot: clap rejects the two together, so the
    // fallback to `--until` only ever sees the other branch.
    let (cutoff_raw, cutoff_flag) = match as_of {
        Some(v) => (Some(v), "--as-of"),
        None => (until, "--until"),
    };
    let since_ts = parse_flag("--since", since, DayEdge::Start)?;
    let cutoff_ts = parse_flag(cutoff_flag, cutoff_raw, DayEdge::End)?;
    // Ordering is checked on the parsed instants, never on the formatted
    // strings: stored precision is mixed and lexicographic compare inverts
    // across precisions.
    validate_order(since.zip(since_ts), cutoff_raw.zip(cutoff_ts), cutoff_flag)?;
    Ok(TimeScope {
        since: since_ts.map(memory_row::iso_format).transpose()?,
        cutoff: cutoff_ts.map(memory_row::iso_format).transpose()?,
        as_of: as_of.is_some(),
    })
}

/// Parse one optional flag value, prefixing [`parse_when`]'s message with
/// the flag name so the user knows which value was rejected.
fn parse_flag(flag: &str, raw: Option<&str>, edge: DayEdge) -> Result<Option<OffsetDateTime>> {
    match raw {
        None => Ok(None),
        Some(v) => match parse_when(v, edge) {
            Ok(ts) => Ok(Some(ts)),
            Err(Error::Usage(msg)) => Err(Error::Usage(format!("{flag}: {msg}"))),
            Err(other) => Err(other),
        },
    }
}

/// Reject an empty window (`--since` after the cutoff), which is always a
/// typo rather than a query that legitimately matches nothing. Each bound
/// arrives paired with the raw value the user typed, so the message quotes
/// that rather than a reformatted instant.
fn validate_order(
    since: Option<(&str, OffsetDateTime)>,
    cutoff: Option<(&str, OffsetDateTime)>,
    cutoff_flag: &str,
) -> Result<()> {
    match (since, cutoff) {
        (Some((s_raw, s)), Some((c_raw, c))) if s > c => Err(Error::Usage(format!(
            "--since must not be later than {cutoff_flag} \
             (got --since `{s_raw}`, {cutoff_flag} `{c_raw}`)"
        ))),
        _ => Ok(()),
    }
}
