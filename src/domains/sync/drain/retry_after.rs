//! `Retry-After`: delay-seconds or an HTTP date (RFC 9110 §10.2.3), capped.

use std::time::Duration;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc2822;

/// The longest delay an upstream may impose; a larger ask is read as this.
pub const CAP: Duration = Duration::from_hours(1);

/// Parse a `Retry-After` value into a delay from now, or `None` when it is
/// neither form.
#[must_use]
pub fn parse(raw: &str) -> Option<Duration> {
    let raw = raw.trim();
    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(Duration::from_secs(seconds).min(CAP));
    }
    // IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`) is RFC 2822 with `GMT`.
    let at = OffsetDateTime::parse(raw, &Rfc2822).ok()?;
    let delta = at - OffsetDateTime::now_utc();
    Some(Duration::try_from(delta).unwrap_or(Duration::ZERO).min(CAP))
}

#[cfg(test)]
#[path = "tests/retry_after.rs"]
mod tests;
