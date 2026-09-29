//! Epoch-millisecond columns on the wire: the platform serializes a `Date`
//! as `toISOString()` (`YYYY-MM-DDTHH:MM:SS.mmmZ`), and a charter's
//! `targetDate` arrives as a calendar date or an RFC 3339 timestamp.

use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{Date, OffsetDateTime, UtcOffset};

use crate::prelude::*;
use crate::utilities::project_error::ProjectError;

/// The current time in epoch milliseconds.
#[must_use]
pub fn now_ms() -> i64 {
    (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// `ms` as `toISOString()` renders it; `None` outside the representable range.
#[must_use]
pub fn iso(ms: i64) -> Option<String> {
    let at = OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).ok()?;
    at.format(format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
    ))
    .ok()
}

/// `targetDate`: `YYYY-MM-DD` is UTC midnight, an RFC 3339 timestamp is
/// converted to UTC; anything else is a schema-edge `400`.
pub fn parse_target_date(value: &str) -> Result<i64> {
    let at = Date::parse(value, format_description!("[year]-[month]-[day]"))
        .map(|date| date.midnight().assume_offset(UtcOffset::UTC))
        .or_else(|_| OffsetDateTime::parse(value, &Rfc3339))
        .map_err(|_| ProjectError::invalid_field("targetDate", "invalid"))?;
    Ok((at.unix_timestamp_nanos() / 1_000_000) as i64)
}

#[cfg(test)]
#[path = "tests/timestamp.rs"]
mod tests;
